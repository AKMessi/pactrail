[CmdletBinding()]
param(
    [ValidateNotNullOrEmpty()]
    [string]$OpenCode = 'opencode',

    [ValidateNotNullOrEmpty()]
    [string]$Model = 'deepseek-direct/deepseek-chat',

    [ValidateNotNullOrEmpty()]
    [string]$ApiKeyEnv = 'DEEPSEEK_API_KEY',

    [ValidateNotNullOrEmpty()]
    [string]$Config = (Join-Path $PSScriptRoot 'opencode-deepseek.json'),

    [ValidateRange(1, 20)]
    [int]$Repetitions = 1,

    [ValidateRange(10, 600)]
    [int]$MaxCaseSeconds = 180,

    [string[]]$CaseId,

    [ValidateNotNullOrEmpty()]
    [string]$OutputDirectory = (Join-Path $PWD 'benchmark-results-opencode'),

    [ValidateNotNullOrEmpty()]
    [string]$WorkspaceDirectory = (Join-Path ([System.IO.Path]::GetTempPath()) 'pactrail-mvb-opencode-workspaces'),

    [ValidateNotNullOrEmpty()]
    [string]$RuntimeDirectory = (Join-Path ([System.IO.Path]::GetTempPath()) 'pactrail-mvb-opencode-runtime')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

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
    if ($missing) { throw "Unknown case id(s): $($missing -join ', ')" }
}
if (-not $cases) { throw 'No benchmark cases selected.' }

$apiKey = [Environment]::GetEnvironmentVariable($ApiKeyEnv, 'Process')
if ([string]::IsNullOrWhiteSpace($apiKey)) {
    $apiKey = [Environment]::GetEnvironmentVariable($ApiKeyEnv, 'User')
}
if ([string]::IsNullOrWhiteSpace($apiKey)) {
    $apiKey = [Environment]::GetEnvironmentVariable($ApiKeyEnv, 'Machine')
}
if ([string]::IsNullOrWhiteSpace($apiKey)) {
    throw "Environment variable '$ApiKeyEnv' is required."
}
[Environment]::SetEnvironmentVariable($ApiKeyEnv, $apiKey, 'Process')

$opencodeCommand = Get-Command $OpenCode -ErrorAction Stop
$configPath = [System.IO.Path]::GetFullPath($Config)
if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
    throw "OpenCode config does not exist: $configPath"
}

$runStamp = [DateTimeOffset]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$modelSlug = $Model -replace '[^A-Za-z0-9._-]', '-'
$resultRoot = Join-Path ([System.IO.Path]::GetFullPath($OutputDirectory)) "$runStamp-$modelSlug"
$workspaceRoot = [System.IO.Path]::GetFullPath($WorkspaceDirectory)
$runtimeRoot = [System.IO.Path]::GetFullPath($RuntimeDirectory)
New-Item -ItemType Directory -Force $resultRoot, $workspaceRoot, $runtimeRoot | Out-Null

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Write-Utf8File {
    param([string]$Path, [AllowEmptyString()][string]$Content)
    $parent = Split-Path -Parent $Path
    if ($parent) { New-Item -ItemType Directory -Force $parent | Out-Null }
    [System.IO.File]::WriteAllText($Path, $Content, $utf8NoBom)
}

function Get-VisibleSnapshot {
    param([string]$Root)
    $snapshot = [ordered]@{}
    if (-not (Test-Path -LiteralPath $Root)) { return $snapshot }
    $files = Get-ChildItem -LiteralPath $Root -Recurse -File -Force | Sort-Object FullName
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($Root.Length).TrimStart('\', '/') -replace '\\', '/'
        $snapshot[$relative] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    return $snapshot
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

function Add-Assertion {
    param([System.Collections.ArrayList]$Assertions, [string]$Name, [bool]$Passed, [string]$Detail)
    [void]$Assertions.Add([pscustomobject]@{ name = $Name; passed = $Passed; detail = $Detail })
}

function Test-CaseWorkspace {
    param([string]$Root, $Baseline, $Case)
    $assertions = New-Object System.Collections.ArrayList
    foreach ($property in $Case.expected_files.PSObject.Properties) {
        $relativePath = $property.Name -replace '/', [System.IO.Path]::DirectorySeparatorChar
        $path = Join-Path $Root $relativePath
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            Add-Assertion $assertions "file:$($property.Name)" $false 'Expected file is missing.'
            continue
        }
        $actual = [System.IO.File]::ReadAllText($path)
        $expected = [string]$property.Value
        $matches = (Normalize-Text $actual $Case.allow_terminal_newline) -ceq
            (Normalize-Text $expected $Case.allow_terminal_newline)
        Add-Assertion $assertions "file:$($property.Name)" $matches $(if ($matches) { 'Content matched.' } else { 'Content differed.' })
    }
    foreach ($relative in $Case.expected_absent) {
        $relativePath = [string]$relative -replace '/', [System.IO.Path]::DirectorySeparatorChar
        $absent = -not (Test-Path -LiteralPath (Join-Path $Root $relativePath))
        Add-Assertion $assertions "absent:$relative" $absent $(if ($absent) { 'Path is absent.' } else { 'Path still exists.' })
    }
    $snapshot = Get-VisibleSnapshot $Root
    $changed = @(Get-ChangedPaths $Baseline $snapshot)
    $expectedChanged = @($Case.expected_changed_paths | ForEach-Object { [string]$_ } | Sort-Object)
    $matchesChanged = @(Compare-Object $expectedChanged $changed).Count -eq 0
    Add-Assertion $assertions 'changed-paths' $matchesChanged "Expected [$($expectedChanged -join ', ')]; observed [$($changed -join ', ')]."
    return [pscustomobject]@{
        passed = @($assertions | Where-Object { -not $_.passed }).Count -eq 0
        changed_paths = $changed
        assertions = @($assertions)
    }
}

function ConvertTo-NativeArgument {
    param([AllowEmptyString()][string]$Value)
    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') { return $Value }
    return '"' + ($Value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
}

function Invoke-OpenCode {
    param([string[]]$Arguments, [string]$StdoutPath, [string]$StderrPath)
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $commandArguments = @()
    if ($opencodeCommand.CommandType -eq 'ExternalScript') {
        $startInfo.FileName = (Get-Process -Id $PID).Path
        $commandArguments += @('-NoLogo', '-NoProfile', '-File', $opencodeCommand.Source)
    } else {
        $startInfo.FileName = $opencodeCommand.Source
    }
    $commandArguments += $Arguments
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.EnvironmentVariables['OPENCODE_CONFIG'] = $configPath
    $startInfo.EnvironmentVariables['OPENCODE_CONFIG_DIR'] = Join-Path $runtimeRoot 'config-dir'
    $startInfo.EnvironmentVariables['XDG_CONFIG_HOME'] = Join-Path $runtimeRoot 'config'
    $startInfo.EnvironmentVariables['XDG_DATA_HOME'] = Join-Path $runtimeRoot 'data'
    $startInfo.EnvironmentVariables['XDG_CACHE_HOME'] = Join-Path $runtimeRoot 'cache'
    $startInfo.EnvironmentVariables[$ApiKeyEnv] = $apiKey
    $startInfo.Arguments = ($commandArguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' '

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw 'Failed to start OpenCode.' }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $completed = $process.WaitForExit($MaxCaseSeconds * 1000)
    if (-not $completed) {
        $process.Kill()
        $process.WaitForExit()
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $exitCode = if ($completed) { $process.ExitCode } else { 124 }
    $process.Dispose()
    $timer.Stop()
    Write-Utf8File $StdoutPath $stdout
    Write-Utf8File $StderrPath $stderr
    return [pscustomobject]@{ ExitCode = $exitCode; DurationMs = $timer.ElapsedMilliseconds; Output = $stdout; TimedOut = -not $completed }
}

function Get-OpenCodeMetrics {
    param([string]$JsonLines)
    $inputTokens = 0L
    $outputTokens = 0L
    $cachedInputTokens = 0L
    $modelCalls = 0
    $toolCalls = 0
    $failedTools = 0
    $errors = 0
    $texts = New-Object System.Collections.Generic.List[string]
    foreach ($line in $JsonLines -split "`r?`n") {
        if (-not $line.Trim()) { continue }
        try { $event = $line | ConvertFrom-Json } catch { $errors++; continue }
        switch ($event.type) {
            'step_finish' {
                $modelCalls++
                $inputTokens += [long]$event.part.tokens.input
                $outputTokens += [long]$event.part.tokens.output
                $cachedInputTokens += [long]$event.part.tokens.cache.read
            }
            'tool_use' {
                $toolCalls++
                if ($event.part.state.status -ne 'completed') { $failedTools++ }
            }
            'text' { [void]$texts.Add([string]$event.part.text) }
            'error' { $errors++ }
        }
    }
    return [pscustomobject]@{
        model_calls = $modelCalls
        tool_calls = $toolCalls
        failed_tool_calls = $failedTools
        input_tokens = $inputTokens + $cachedInputTokens
        uncached_input_tokens = $inputTokens
        cached_input_tokens = $cachedInputTokens
        output_tokens = $outputTokens
        total_tokens = $inputTokens + $cachedInputTokens + $outputTokens
        errors = $errors
        summary = $texts -join "`n"
    }
}

New-Item -ItemType Directory -Force (Join-Path $runtimeRoot 'config-dir') | Out-Null
$results = New-Object System.Collections.ArrayList
foreach ($case in $cases) {
    for ($repetition = 1; $repetition -le $Repetitions; $repetition++) {
        $caseKey = if ($Repetitions -eq 1) { $case.id } else { "$($case.id)-r$repetition" }
        Write-Host "[$Model via OpenCode] $caseKey"
        $workspace = Join-Path $workspaceRoot "$runStamp-$modelSlug-$caseKey"
        $artifactDirectory = Join-Path $resultRoot $caseKey
        if (Test-Path -LiteralPath $workspace) { Remove-Item -LiteralPath $workspace -Recurse -Force }
        New-Item -ItemType Directory -Force $workspace, $artifactDirectory | Out-Null
        foreach ($property in $case.initial_files.PSObject.Properties) {
            $relativePath = $property.Name -replace '/', [System.IO.Path]::DirectorySeparatorChar
            Write-Utf8File (Join-Path $workspace $relativePath) ([string]$property.Value)
        }
        $before = Get-VisibleSnapshot $workspace
        $invoke = Invoke-OpenCode @('run', '--dir', $workspace, '--model', $Model, '--agent', 'build', '--format', 'json', [string]$case.prompt) `
            (Join-Path $artifactDirectory 'run-output.jsonl') (Join-Path $artifactDirectory 'run-stderr.txt')
        $metrics = Get-OpenCodeMetrics $invoke.Output
        $grade = Test-CaseWorkspace $workspace $before $case
        $assertions = New-Object System.Collections.ArrayList
        foreach ($assertion in $grade.assertions) { [void]$assertions.Add($assertion) }
        foreach ($term in $case.expected_summary_terms) {
            $hasTerm = $metrics.summary.IndexOf([string]$term, [StringComparison]::OrdinalIgnoreCase) -ge 0
            Add-Assertion $assertions "summary:$term" $hasTerm $(if ($hasTerm) { 'Summary contained the term.' } else { 'Summary omitted the term.' })
        }
        Add-Assertion $assertions 'provider-errors' ($metrics.errors -eq 0) "Observed $($metrics.errors) error event(s)."
        Add-Assertion $assertions 'case-timeout' (-not $invoke.TimedOut) $(if ($invoke.TimedOut) { 'The case exceeded its wall-time limit.' } else { 'The case completed within its wall-time limit.' })
        $passed = @($assertions | Where-Object { -not $_.passed }).Count -eq 0
        $result = [pscustomobject]@{
            case_id = [string]$case.id
            repetition = $repetition
            category = [string]$case.category
            model = $Model
            harness = 'opencode'
            passed = $passed
            exit_code = $invoke.ExitCode
            timed_out = $invoke.TimedOut
            duration_ms = $invoke.DurationMs
            changed_paths = $grade.changed_paths
            direct_source_write = $grade.changed_paths.Count -gt 0
            source_unchanged_after_run = $grade.changed_paths.Count -eq 0
            metrics = $metrics
            assertions = @($assertions)
            summary = $metrics.summary
        }
        [void]$results.Add($result)
        Write-Utf8File (Join-Path $artifactDirectory 'result.json') ($result | ConvertTo-Json -Depth 8)
    }
}

$passedCount = @($results | Where-Object { $_.passed }).Count
$durations = @($results | ForEach-Object { [long]$_.duration_ms } | Sort-Object)
$middle = [int][Math]::Floor($durations.Count / 2)
$median = if ($durations.Count % 2 -eq 0) { [long](($durations[$middle - 1] + $durations[$middle]) / 2) } else { $durations[$middle] }
$version = (& $opencodeCommand.Source --version 2>&1 | Out-String).Trim()
$summaryObject = [pscustomobject]@{
    schema_version = 1
    suite = [string]$manifest.suite
    started_at = $runStamp
    harness = 'opencode'
    harness_version = $version
    model = $Model
    repetitions = $Repetitions
    max_case_seconds = $MaxCaseSeconds
    cases = $results.Count
    passed = $passedCount
    failed = $results.Count - $passedCount
    pass_rate = [Math]::Round($passedCount / $results.Count, 4)
    median_duration_ms = $median
    total_tokens = [long](($results | ForEach-Object { $_.metrics.total_tokens } | Measure-Object -Sum).Sum)
    direct_source_writes = @($results | Where-Object { $_.direct_source_write }).Count
    results = @($results)
}
Write-Utf8File (Join-Path $resultRoot 'summary.json') ($summaryObject | ConvertTo-Json -Depth 10)

$markdown = New-Object System.Collections.Generic.List[string]
$markdown.Add("# OpenCode comparator - $Model")
$markdown.Add('')
$markdown.Add("- Result: **$passedCount/$($results.Count) passed** ($([Math]::Round($summaryObject.pass_rate * 100, 1))%)")
$markdown.Add("- OpenCode: ``$version``")
$markdown.Add("- Median end-to-end task time: $([Math]::Round($median / 1000, 2)) s")
$markdown.Add("- Total reported model tokens: $($summaryObject.total_tokens)")
$markdown.Add("- Cases that wrote directly to the source workspace: $($summaryObject.direct_source_writes)/$($results.Count)")
$markdown.Add('')
$markdown.Add('| Case | Result | Time | Tokens | Model/tool calls | Direct write |')
$markdown.Add('|---|---:|---:|---:|---:|---:|')
foreach ($result in $results) {
    $markdown.Add("| $($result.case_id) r$($result.repetition) | $(if ($result.passed) { 'PASS' } else { 'FAIL' }) | $([Math]::Round($result.duration_ms / 1000, 2)) s | $($result.metrics.total_tokens) | $($result.metrics.model_calls)/$($result.metrics.tool_calls) | $($result.direct_source_write) |")
}
$markdown.Add('')
$markdown.Add('This comparator scores exact final workspace artifacts and required summary terms. OpenCode edits the source workspace directly, so Pactrail-only candidate, apply-boundary, receipt, and hash-chain assertions are reported as architectural differences rather than counted as OpenCode task failures.')
Write-Utf8File (Join-Path $resultRoot 'SUMMARY.md') ($markdown -join "`n")

Write-Host ''
Write-Host "$passedCount/$($results.Count) cases passed. Results: $resultRoot"
if ($passedCount -ne $results.Count) { exit 2 }
