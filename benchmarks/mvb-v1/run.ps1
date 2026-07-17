[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$Model,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^https?://')]
    [string]$BaseUrl,

    [ValidateNotNullOrEmpty()]
    [string]$ApiKeyEnv = 'PACTRAIL_LOCAL_API_KEY',

    [ValidateNotNullOrEmpty()]
    [string]$Pactrail = 'pactrail',

    [ValidateRange(1024, 131072)]
    [int]$ContextTokens = 4096,

    [ValidateRange(1, 131071)]
    [int]$MaxOutputTokens = 512,

    [ValidateRange(1, 256)]
    [int]$MaxTurns = 12,

    [ValidateRange(1, 20)]
    [int]$Repetitions = 1,

    [string[]]$CaseId,

    [ValidateNotNullOrEmpty()]
    [string]$OutputDirectory = (Join-Path $PWD 'benchmark-results'),

    [ValidateNotNullOrEmpty()]
    [string]$WorkspaceDirectory = (Join-Path ([System.IO.Path]::GetTempPath()) 'pactrail-mvb-workspaces')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($MaxOutputTokens -ge $ContextTokens) {
    throw 'MaxOutputTokens must be smaller than ContextTokens.'
}

$suiteRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$manifest = Get-Content -LiteralPath (Join-Path $suiteRoot 'cases.json') -Raw | ConvertFrom-Json
if ($manifest.schema_version -ne 1) {
    throw "Unsupported benchmark schema version: $($manifest.schema_version)"
}

$cases = @($manifest.cases)
if ($null -ne $CaseId) {
    $requested = @{}
    foreach ($id in $CaseId) { $requested[$id] = $true }
    $cases = @($cases | Where-Object { $requested.ContainsKey($_.id) })
    $selectedIds = @($cases | ForEach-Object { $_.id })
    $missing = @($CaseId | Where-Object { $selectedIds -notcontains $_ })
    if ($missing) {
        throw "Unknown case id(s): $($missing -join ', ')"
    }
}
if (-not $cases) {
    throw 'No benchmark cases selected.'
}

$pactrailCommand = Get-Command $Pactrail -ErrorAction Stop
if (-not [Environment]::GetEnvironmentVariable($ApiKeyEnv)) {
    [Environment]::SetEnvironmentVariable($ApiKeyEnv, 'local', 'Process')
}

$modelSlug = $Model -replace '[^A-Za-z0-9._-]', '-'
$runStamp = [DateTimeOffset]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$resultRoot = Join-Path ([System.IO.Path]::GetFullPath($OutputDirectory)) "$runStamp-$modelSlug"
New-Item -ItemType Directory -Force -Path $resultRoot | Out-Null
New-Item -ItemType Directory -Force -Path $WorkspaceDirectory | Out-Null

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Write-Utf8File {
    param([string]$Path, [AllowEmptyString()][string]$Content)
    $parent = Split-Path -Parent $Path
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    [System.IO.File]::WriteAllText($Path, $Content, $utf8NoBom)
}

function Get-VisibleSnapshot {
    param([string]$Root)
    $snapshot = [ordered]@{}
    if (-not (Test-Path -LiteralPath $Root)) { return $snapshot }
    $stateRoot = Join-Path $Root '.pactrail'
    $files = Get-ChildItem -LiteralPath $Root -Recurse -File -Force |
        Where-Object { $_.FullName -notlike "$stateRoot*" } |
        Sort-Object FullName
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($Root.Length).TrimStart('\', '/') -replace '\\', '/'
        $snapshot[$relative] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    return $snapshot
}

function Test-SnapshotEqual {
    param($Left, $Right)
    if ($Left.Count -ne $Right.Count) { return $false }
    foreach ($key in $Left.Keys) {
        if (-not $Right.Contains($key) -or $Left[$key] -ne $Right[$key]) { return $false }
    }
    return $true
}

function Get-ChangedPaths {
    param($Before, $After)
    $all = @($Before.Keys) + @($After.Keys) | Sort-Object -Unique
    return @($all | Where-Object {
        -not $Before.Contains($_) -or -not $After.Contains($_) -or $Before[$_] -ne $After[$_]
    })
}

function Normalize-Text {
    param([string]$Text, [bool]$AllowTerminalNewline)
    $normalized = $Text -replace "`r`n", "`n"
    if ($AllowTerminalNewline -and $normalized.EndsWith("`n")) {
        return $normalized.Substring(0, $normalized.Length - 1)
    }
    return $normalized
}

function ConvertTo-NativeArgument {
    param([AllowEmptyString()][string]$Value)
    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') { return $Value }
    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $backslashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') {
            $backslashes++
        } elseif ($character -eq '"') {
            [void]$builder.Append(('\' * (($backslashes * 2) + 1)))
            [void]$builder.Append('"')
            $backslashes = 0
        } else {
            if ($backslashes -gt 0) { [void]$builder.Append(('\' * $backslashes)) }
            [void]$builder.Append($character)
            $backslashes = 0
        }
    }
    if ($backslashes -gt 0) { [void]$builder.Append(('\' * ($backslashes * 2))) }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Invoke-Pactrail {
    param([string[]]$Arguments, [string]$StdoutPath, [string]$StderrPath)
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $pactrailCommand.Source
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    if ($startInfo.PSObject.Properties.Name -contains 'ArgumentList') {
        foreach ($argument in $Arguments) { [void]$startInfo.ArgumentList.Add($argument) }
    } else {
        $startInfo.Arguments = ($Arguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' '
    }
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw 'Failed to start Pactrail.' }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $exitCode = $process.ExitCode
    $process.Dispose()
    $timer.Stop()
    Write-Utf8File -Path $StdoutPath -Content $stdout
    Write-Utf8File -Path $StderrPath -Content $stderr
    return [pscustomobject]@{
        ExitCode = $exitCode
        DurationMs = $timer.ElapsedMilliseconds
        Output = $stdout
    }
}

function ConvertFrom-PactrailJson {
    param([string]$Text)
    try { return $Text | ConvertFrom-Json } catch { return $null }
}

function Add-Assertion {
    param([System.Collections.ArrayList]$Assertions, [string]$Name, [bool]$Passed, [string]$Detail)
    [void]$Assertions.Add([pscustomobject]@{ name = $Name; passed = $Passed; detail = $Detail })
}

function Get-TraceMetrics {
    param([string]$TracePath)
    $modelCalls = 0
    $toolCalls = 0
    $failedTools = 0
    $modelDurationMs = 0L
    $toolDurationMs = 0L
    $inputTokens = 0L
    $outputTokens = 0L
    $recoveries = 0
    $events = 0
    if (Test-Path -LiteralPath $TracePath) {
        foreach ($line in Get-Content -LiteralPath $TracePath) {
            if (-not $line.Trim()) { continue }
            $event = $line | ConvertFrom-Json
            $events++
            if ($event.event.type -eq 'action_completed') {
                $data = $event.event.data
                if ($data.actor -like 'model:*') {
                    $modelCalls++
                    $modelDurationMs += [long]$data.duration_ms
                    if ($data.attributes.PSObject.Properties.Name -contains 'input_tokens') {
                        $inputTokens += [long]$data.attributes.input_tokens
                    }
                    if ($data.attributes.PSObject.Properties.Name -contains 'output_tokens') {
                        $outputTokens += [long]$data.attributes.output_tokens
                    }
                } elseif ($data.actor -like 'tool:*') {
                    $toolCalls++
                    $toolDurationMs += [long]$data.duration_ms
                    if (-not $data.succeeded) { $failedTools++ }
                }
            } elseif ($event.event.type -eq 'note_recorded' -and
                      $event.event.data.message -like '*recovery stopped*') {
                $recoveries++
            }
        }
    }
    return [pscustomobject]@{
        events = $events
        model_calls = $modelCalls
        tool_calls = $toolCalls
        failed_tool_calls = $failedTools
        model_duration_ms = $modelDurationMs
        tool_duration_ms = $toolDurationMs
        input_tokens = $inputTokens
        output_tokens = $outputTokens
        recovery_stops = $recoveries
    }
}

try {
    $modelsResponse = Invoke-RestMethod -Uri ($BaseUrl.TrimEnd('/') + '/models') -TimeoutSec 10
} catch {
    throw "The model endpoint is not ready at ${BaseUrl}: $($_.Exception.Message)"
}

$results = New-Object System.Collections.ArrayList
foreach ($case in $cases) {
    for ($repetition = 1; $repetition -le $Repetitions; $repetition++) {
        $caseKey = if ($Repetitions -eq 1) { $case.id } else { "$($case.id)-r$repetition" }
        Write-Host "[$Model] $caseKey"
        $workspace = Join-Path ([System.IO.Path]::GetFullPath($WorkspaceDirectory)) "$runStamp-$modelSlug-$caseKey"
        $artifactDirectory = Join-Path $resultRoot $caseKey
        if (Test-Path -LiteralPath $workspace) { Remove-Item -LiteralPath $workspace -Recurse -Force }
        New-Item -ItemType Directory -Force -Path $workspace | Out-Null
        New-Item -ItemType Directory -Force -Path $artifactDirectory | Out-Null

        foreach ($property in $case.initial_files.PSObject.Properties) {
            $relativePath = $property.Name -replace '/', [System.IO.Path]::DirectorySeparatorChar
            Write-Utf8File -Path (Join-Path $workspace $relativePath) -Content ([string]$property.Value)
        }
        $before = Get-VisibleSnapshot -Root $workspace

        $arguments = @(
            'run', '--workspace', $workspace,
            '--provider', 'open-ai-compatible',
            '--base-url', $BaseUrl,
            '--model', $Model,
            '--api-key-env', $ApiKeyEnv,
            '--context-tokens', $ContextTokens.ToString(),
            '--max-output-tokens', $MaxOutputTokens.ToString(),
            '--max-turns', $MaxTurns.ToString(),
            '--output', 'json'
        )
        foreach ($writePath in $case.write_paths) {
            $arguments += @('--write-path', [string]$writePath)
        }
        $arguments += [string]$case.prompt

        $invoke = Invoke-Pactrail -Arguments $arguments `
            -StdoutPath (Join-Path $artifactDirectory 'run-output.json') `
            -StderrPath (Join-Path $artifactDirectory 'run-stderr.txt')
        $runJson = ConvertFrom-PactrailJson -Text $invoke.Output
        $runId = if ($null -ne $runJson) { [string]$runJson.run_id } else { '' }
        if (-not $runId) {
            $errorText = Get-Content -LiteralPath (Join-Path $artifactDirectory 'run-stderr.txt') -Raw -ErrorAction SilentlyContinue
            if ($errorText -match 'run ([0-9a-f-]{36})') { $runId = $Matches[1] }
        }

        $candidateSnapshot = Get-VisibleSnapshot -Root $workspace
        $isolatedBeforeApply = Test-SnapshotEqual -Left $before -Right $candidateSnapshot
        $applyExitCode = $null
        if ($null -ne $runJson -and $runJson.outcome -eq 'ready_to_apply' -and $runId) {
            $apply = Invoke-Pactrail -Arguments @('apply', '--workspace', $workspace, '--json', $runId) `
                -StdoutPath (Join-Path $artifactDirectory 'apply-output.json') `
                -StderrPath (Join-Path $artifactDirectory 'apply-stderr.txt')
            $applyExitCode = $apply.ExitCode
        }

        $after = Get-VisibleSnapshot -Root $workspace
        $assertions = New-Object System.Collections.ArrayList
        Add-Assertion $assertions 'transaction-isolation' $isolatedBeforeApply 'The source workspace was unchanged before apply.'

        foreach ($property in $case.expected_files.PSObject.Properties) {
            $relativePath = $property.Name -replace '/', [System.IO.Path]::DirectorySeparatorChar
            $path = Join-Path $workspace $relativePath
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
                Add-Assertion $assertions "file:$($property.Name)" $false 'Expected file is missing.'
                continue
            }
            $actual = [System.IO.File]::ReadAllText($path)
            $expected = [string]$property.Value
            $matches = (Normalize-Text $actual $case.allow_terminal_newline) -ceq
                (Normalize-Text $expected $case.allow_terminal_newline)
            $detail = if ($matches) { 'Content matched.' } else { 'Content differed.' }
            Add-Assertion $assertions "file:$($property.Name)" $matches $detail
        }
        foreach ($relative in $case.expected_absent) {
            $relativePath = [string]$relative -replace '/', [System.IO.Path]::DirectorySeparatorChar
            $absent = -not (Test-Path -LiteralPath (Join-Path $workspace $relativePath))
            $detail = if ($absent) { 'Path is absent.' } else { 'Path still exists.' }
            Add-Assertion $assertions "absent:$relative" $absent $detail
        }

        $actualChanged = @(Get-ChangedPaths -Before $before -After $after)
        $expectedChanged = @($case.expected_changed_paths | ForEach-Object { [string]$_ } | Sort-Object)
        $changedMatches = @(
            Compare-Object -ReferenceObject @($expectedChanged) -DifferenceObject @($actualChanged)
        ).Count -eq 0
        Add-Assertion $assertions 'changed-paths' $changedMatches "Expected [$($expectedChanged -join ', ')]; observed [$($actualChanged -join ', ')]."

        $summary = if ($null -ne $runJson) { [string]$runJson.summary } else { '' }
        foreach ($term in $case.expected_summary_terms) {
            $hasTerm = $summary.IndexOf([string]$term, [StringComparison]::OrdinalIgnoreCase) -ge 0
            $detail = if ($hasTerm) { 'Summary contained the term.' } else { 'Summary omitted the term.' }
            Add-Assertion $assertions "summary:$term" $hasTerm $detail
        }

        $traceValid = $false
        $tracePath = ''
        if ($runId) {
            $traceCheck = Invoke-Pactrail -Arguments @('trace', '--workspace', $workspace, '--json', $runId) `
                -StdoutPath (Join-Path $artifactDirectory 'trace-render.json') `
                -StderrPath (Join-Path $artifactDirectory 'trace-stderr.txt')
            $traceValid = $traceCheck.ExitCode -eq 0 -and $null -ne (ConvertFrom-PactrailJson -Text $traceCheck.Output)
            $runDirectory = Join-Path $workspace ".pactrail/runs/$runId"
            $tracePath = Join-Path $runDirectory 'trace.jsonl'
            $receiptPath = Join-Path $runDirectory 'receipt.json'
            if (Test-Path -LiteralPath $tracePath) {
                Copy-Item -LiteralPath $tracePath -Destination (Join-Path $artifactDirectory 'trace.jsonl')
            }
            if (Test-Path -LiteralPath $receiptPath) {
                Copy-Item -LiteralPath $receiptPath -Destination (Join-Path $artifactDirectory 'receipt.json')
            }
        }
        $traceDetail = if ($traceValid) { 'Pactrail accepted the hash-chained trace.' } else { 'Trace validation was unavailable or failed.' }
        Add-Assertion $assertions 'trace-integrity' $traceValid $traceDetail

        $metrics = Get-TraceMetrics -TracePath $tracePath
        $reportedTokens = if ($null -ne $runJson) {
            [long]$runJson.tokens
        } else {
            $metrics.input_tokens + $metrics.output_tokens
        }
        $passed = @($assertions | Where-Object { -not $_.passed }).Count -eq 0
        $result = [pscustomobject]@{
            case_id = [string]$case.id
            repetition = $repetition
            category = [string]$case.category
            model = $Model
            passed = $passed
            pactrail_exit_code = $invoke.ExitCode
            apply_exit_code = $applyExitCode
            outcome = if ($null -ne $runJson) { [string]$runJson.outcome } else { 'process_error' }
            run_id = $runId
            duration_ms = $invoke.DurationMs
            tokens = $reportedTokens
            isolation_preserved = $isolatedBeforeApply
            trace_integrity_verified = $traceValid
            changed_paths = $actualChanged
            metrics = $metrics
            assertions = @($assertions)
            summary = $summary
        }
        [void]$results.Add($result)
        Write-Utf8File -Path (Join-Path $artifactDirectory 'result.json') -Content ($result | ConvertTo-Json -Depth 8)
    }
}

$passedCount = @($results | Where-Object { $_.passed }).Count
$durationValues = @($results | ForEach-Object { [long]$_.duration_ms } | Sort-Object)
$middle = [int][Math]::Floor($durationValues.Count / 2)
$medianDuration = if ($durationValues.Count % 2 -eq 0) {
    [long](($durationValues[$middle - 1] + $durationValues[$middle]) / 2)
} else {
    $durationValues[$middle]
}

$pactrailVersion = (& $pactrailCommand.Source --version 2>&1 | Out-String).Trim()
$summaryObject = [pscustomobject]@{
    schema_version = 1
    suite = [string]$manifest.suite
    started_at = $runStamp
    model = $Model
    base_url = $BaseUrl
    pactrail_version = $pactrailVersion
    context_tokens = $ContextTokens
    max_output_tokens = $MaxOutputTokens
    max_turns = $MaxTurns
    repetitions = $Repetitions
    cases = $results.Count
    passed = $passedCount
    failed = $results.Count - $passedCount
    pass_rate = [Math]::Round($passedCount / $results.Count, 4)
    median_duration_ms = $medianDuration
    total_tokens = [long](($results | Measure-Object -Property tokens -Sum).Sum)
    endpoint_models = @($modelsResponse.data | ForEach-Object { $_.id })
    results = @($results)
}
Write-Utf8File -Path (Join-Path $resultRoot 'summary.json') -Content ($summaryObject | ConvertTo-Json -Depth 10)

$markdown = New-Object System.Collections.Generic.List[string]
$markdown.Add("# Pactrail MVB v1 - $Model")
$markdown.Add('')
$markdown.Add("- Result: **$passedCount/$($results.Count) passed** ($([Math]::Round($summaryObject.pass_rate * 100, 1))%)")
$markdown.Add("- Pactrail: ``$pactrailVersion``")
$markdown.Add("- Context/output/turns: $ContextTokens / $MaxOutputTokens / $MaxTurns")
$markdown.Add("- Median end-to-end task time: $([Math]::Round($medianDuration / 1000, 2)) s")
$markdown.Add("- Total reported model tokens: $($summaryObject.total_tokens)")
$markdown.Add('')
$markdown.Add('| Case | Category | Result | Time | Tokens | Model/tool calls | Recovery stop |')
$markdown.Add('|---|---|---:|---:|---:|---:|---:|')
foreach ($result in $results) {
    $mark = if ($result.passed) { 'PASS' } else { 'FAIL' }
    $markdown.Add("| $($result.case_id) | $($result.category) | $mark | $([Math]::Round($result.duration_ms / 1000, 2)) s | $($result.tokens) | $($result.metrics.model_calls)/$($result.metrics.tool_calls) | $($result.metrics.recovery_stops) |")
}
$markdown.Add('')
$markdown.Add("A pass requires exact final workspace assertions, transaction isolation before apply, and a trace accepted by Pactrail's integrity checker. See ``summary.json`` and each case directory for raw outputs, receipts, and portable JSONL traces.")
Write-Utf8File -Path (Join-Path $resultRoot 'SUMMARY.md') -Content ($markdown -join "`n")

Write-Host ''
Write-Host "$passedCount/$($results.Count) cases passed. Results: $resultRoot"
if ($passedCount -ne $results.Count) { exit 2 }
