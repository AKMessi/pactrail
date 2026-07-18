[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('pactrail', 'opencode')]
    [string]$Harness,

    [Parameter(Mandatory = $true)]
    [ValidateSet('deepseek-v4-flash', 'deepseek-v4-pro')]
    [string]$Model,

    [ValidateNotNullOrEmpty()]
    [string]$Pactrail = (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'target/release/pactrail.exe'),

    [ValidateNotNullOrEmpty()]
    [string]$OpenCode = 'opencode',

    [ValidateNotNullOrEmpty()]
    [string]$ApiKeyEnv = 'DEEPSEEK_API_KEY',

    [string[]]$CaseId,

    [switch]$ValidateGraders,

    [switch]$KeepWorkspaces,

    [ValidateNotNullOrEmpty()]
    [string]$OutputDirectory = (Join-Path $PWD 'benchmark-results/issue-replay-v1'),

    [ValidateNotNullOrEmpty()]
    [string]$WorkspaceDirectory = 'D:/AKMESSI/CODING/AI/Projects/pactrail-benchmark-work/scored',

    [ValidateNotNullOrEmpty()]
    [string]$CacheDirectory = 'D:/AKMESSI/CODING/AI/Projects/pactrail-benchmark-work/cache'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$suiteRoot = $PSScriptRoot
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$manifest = Get-Content -LiteralPath (Join-Path $suiteRoot 'cases.json') -Raw | ConvertFrom-Json
if ($manifest.schema_version -ne 1) {
    throw "Unsupported issue-replay manifest version: $($manifest.schema_version)"
}

$cases = @($manifest.tasks)
if ($null -ne $CaseId) {
    $requested = @{}
    foreach ($id in $CaseId) { $requested[$id] = $true }
    $cases = @($cases | Where-Object { $requested.ContainsKey($_.id) })
    $selected = @($cases | ForEach-Object { [string]$_.id })
    $missing = @($CaseId | Where-Object { $selected -notcontains $_ })
    if ($missing) { throw "Unknown case id(s): $($missing -join ', ')" }
}
if (-not $cases) { throw 'No issue-replay cases were selected.' }

function Write-Utf8File {
    param([string]$Path, [AllowEmptyString()][string]$Content)
    $parent = Split-Path -Parent $Path
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    [System.IO.File]::WriteAllText($Path, $Content, $utf8NoBom)
}

function ConvertTo-NativeArgument {
    param([AllowEmptyString()][string]$Value)
    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') { return $Value }
    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $slashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') {
            $slashes++
        } elseif ($character -eq '"') {
            [void]$builder.Append(('\' * (($slashes * 2) + 1)))
            [void]$builder.Append('"')
            $slashes = 0
        } else {
            if ($slashes -gt 0) { [void]$builder.Append(('\' * $slashes)) }
            [void]$builder.Append($character)
            $slashes = 0
        }
    }
    if ($slashes -gt 0) { [void]$builder.Append(('\' * ($slashes * 2))) }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Invoke-CapturedProcess {
    param(
        [string]$FileName,
        [string[]]$Arguments,
        [string]$WorkingDirectory,
        [int]$TimeoutSeconds,
        [string]$StdoutPath,
        [string]$StderrPath,
        [hashtable]$Environment = @{}
    )
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FileName
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    if ($startInfo.PSObject.Properties.Name -contains 'ArgumentList') {
        foreach ($argument in $Arguments) { [void]$startInfo.ArgumentList.Add($argument) }
    } else {
        $startInfo.Arguments = ($Arguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' '
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        $startInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
    }

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) { throw "Failed to start $FileName" }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $completed = $process.WaitForExit($TimeoutSeconds * 1000)
    if (-not $completed) {
        try { $process.Kill() } catch { }
        $process.WaitForExit()
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $exitCode = if ($completed) { $process.ExitCode } else { 124 }
    $process.Dispose()
    $timer.Stop()
    if ($StdoutPath) { Write-Utf8File -Path $StdoutPath -Content $stdout }
    if ($StderrPath) { Write-Utf8File -Path $StderrPath -Content $stderr }
    return [pscustomobject]@{
        exit_code = $exitCode
        timed_out = -not $completed
        duration_ms = $timer.ElapsedMilliseconds
        stdout = $stdout
        stderr = $stderr
    }
}

function Invoke-Git {
    param([string]$WorkingDirectory, [string[]]$Arguments, [int]$TimeoutSeconds = 120)
    $result = Invoke-CapturedProcess -FileName 'git' -Arguments $Arguments -WorkingDirectory $WorkingDirectory `
        -TimeoutSeconds $TimeoutSeconds -StdoutPath '' -StderrPath ''
    if ($result.exit_code -ne 0) {
        throw "git $($Arguments -join ' ') failed: $($result.stderr.Trim())"
    }
    return $result.stdout
}

function Get-ApiKey {
    $key = [Environment]::GetEnvironmentVariable($ApiKeyEnv, 'Process')
    if ([string]::IsNullOrWhiteSpace($key)) { $key = [Environment]::GetEnvironmentVariable($ApiKeyEnv, 'User') }
    if ([string]::IsNullOrWhiteSpace($key)) { $key = [Environment]::GetEnvironmentVariable($ApiKeyEnv, 'Machine') }
    if ([string]::IsNullOrWhiteSpace($key)) { throw "Environment variable '$ApiKeyEnv' is required." }
    [Environment]::SetEnvironmentVariable($ApiKeyEnv, $key, 'Process')
    return $key
}

function Get-DeepSeekBalance {
    param([string]$ApiKey)
    $headers = @{ Authorization = "Bearer $ApiKey" }
    $response = Invoke-RestMethod -Uri 'https://api.deepseek.com/user/balance' -Headers $headers -Method Get -TimeoutSec 20
    $usd = $response.balance_infos | Where-Object { $_.currency -eq 'USD' } | Select-Object -First 1
    if ($null -eq $usd) { throw 'DeepSeek balance response did not include USD.' }
    return [decimal]$usd.total_balance
}

function Assert-SafeChildPath {
    param([string]$Root, [string]$Path)
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    $pathFull = [System.IO.Path]::GetFullPath($Path)
    if (-not $pathFull.StartsWith($rootFull, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing filesystem mutation outside benchmark root: $pathFull"
    }
}

function Reset-BenchmarkDirectory {
    param([string]$Root, [string]$Path)
    Assert-SafeChildPath -Root $Root -Path $Path
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

function Ensure-RepositoryCache {
    param($Case)
    $repositories = Join-Path ([System.IO.Path]::GetFullPath($CacheDirectory)) 'repositories'
    New-Item -ItemType Directory -Force -Path $repositories | Out-Null
    $repoPath = Join-Path $repositories ([string]$Case.id)
    if (-not (Test-Path -LiteralPath (Join-Path $repoPath '.git'))) {
        Reset-BenchmarkDirectory -Root $repositories -Path $repoPath
        $clone = Invoke-CapturedProcess -FileName 'git' -Arguments @('clone', '--quiet', '--filter=blob:none', '--no-checkout', [string]$Case.repository_url, $repoPath) `
            -WorkingDirectory $repositories -TimeoutSeconds 300 -StdoutPath '' -StderrPath ''
        if ($clone.exit_code -ne 0) { throw "Could not clone $($Case.repository): $($clone.stderr)" }
    }
    foreach ($revision in @([string]$Case.base_commit, [string]$Case.reference_commit)) {
        $probe = Invoke-CapturedProcess -FileName 'git' -Arguments @('-C', $repoPath, 'cat-file', '-e', "$revision^{commit}") `
            -WorkingDirectory $repositories -TimeoutSeconds 30 -StdoutPath '' -StderrPath ''
        if ($probe.exit_code -ne 0) {
            $fetch = Invoke-CapturedProcess -FileName 'git' -Arguments @('-C', $repoPath, 'fetch', '--quiet', 'origin', $revision) `
                -WorkingDirectory $repositories -TimeoutSeconds 300 -StdoutPath '' -StderrPath ''
            if ($fetch.exit_code -ne 0) { throw "Could not fetch pinned revision $revision for $($Case.repository)." }
        }
    }
    return $repoPath
}

function Export-Revision {
    param([string]$Repository, [string]$Revision, [string]$Destination, [string]$MutationRoot)
    Reset-BenchmarkDirectory -Root $MutationRoot -Path $Destination
    $archive = "$Destination.zip"
    Assert-SafeChildPath -Root $MutationRoot -Path $archive
    if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
    [void](Invoke-Git -WorkingDirectory $Repository -Arguments @('archive', '--format=zip', "--output=$archive", $Revision) -TimeoutSeconds 180)
    Expand-Archive -LiteralPath $archive -DestinationPath $Destination -Force
    Remove-Item -LiteralPath $archive -Force
}

function Apply-SetupPatch {
    param($Case, [string]$Root)
    if ($null -eq $Case.setup_patch -or [string]::IsNullOrWhiteSpace([string]$Case.setup_patch)) { return }
    $patchPath = Join-Path $suiteRoot ([string]$Case.setup_patch)
    $apply = Invoke-CapturedProcess -FileName 'git' -Arguments @('apply', '--unidiff-zero', '--whitespace=nowarn', $patchPath) `
        -WorkingDirectory $Root -TimeoutSeconds 60 -StdoutPath '' -StderrPath ''
    if ($apply.exit_code -ne 0) { throw "Setup patch failed for $($Case.id): $($apply.stderr)" }
}

function Prepare-CargoControls {
    param($Case, [string]$Root)
    $lock = Join-Path $Root 'Cargo.lock'
    if (-not (Test-Path -LiteralPath $lock)) {
        $generated = Invoke-CapturedProcess -FileName 'cargo' -Arguments @('generate-lockfile') -WorkingDirectory $Root `
            -TimeoutSeconds 300 -StdoutPath '' -StderrPath ''
        if ($generated.exit_code -ne 0) { throw "Could not generate Cargo.lock for $($Case.id): $($generated.stderr)" }
    }
    $fetched = Invoke-CapturedProcess -FileName 'cargo' -Arguments @('fetch', '--locked') -WorkingDirectory $Root `
        -TimeoutSeconds 600 -StdoutPath '' -StderrPath ''
    if ($fetched.exit_code -ne 0) { throw "Could not prefetch dependencies for $($Case.id): $($fetched.stderr)" }
    $config = "[net]`noffline = true`n"
    Write-Utf8File -Path (Join-Path $Root '.cargo/config.toml') -Content $config
}

function Initialize-SyntheticRepository {
    param([string]$Root, $Case)
    [void](Invoke-Git -WorkingDirectory $Root -Arguments @('init', '--quiet'))
    [void](Invoke-Git -WorkingDirectory $Root -Arguments @('config', 'user.name', 'Pactrail Benchmark'))
    [void](Invoke-Git -WorkingDirectory $Root -Arguments @('config', 'user.email', 'benchmark@pactrail.invalid'))
    [void](Invoke-Git -WorkingDirectory $Root -Arguments @('config', 'core.autocrlf', 'false'))
    [void](Invoke-Git -WorkingDirectory $Root -Arguments @('config', 'core.filemode', 'false'))
    [void](Invoke-Git -WorkingDirectory $Root -Arguments @('add', '--all'))
    [void](Invoke-Git -WorkingDirectory $Root -Arguments @('commit', '--quiet', '-m', "benchmark baseline: $($Case.id)"))
}

function Ensure-Template {
    param($Case)
    $templates = Join-Path ([System.IO.Path]::GetFullPath($CacheDirectory)) 'templates'
    New-Item -ItemType Directory -Force -Path $templates | Out-Null
    $template = Join-Path $templates ([string]$Case.id)
    $markerPath = Join-Path $templates "$($Case.id).json"
    $expectedMarker = [ordered]@{
        control_version = 2
        base_commit = [string]$Case.base_commit
        setup_patch = if ($null -eq $Case.setup_patch) { '' } else { [string]$Case.setup_patch }
        setup_patch_sha256 = if ($null -eq $Case.setup_patch) { '' } else {
            (Get-FileHash -LiteralPath (Join-Path $suiteRoot ([string]$Case.setup_patch)) -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $markerMatches = $false
    if ((Test-Path -LiteralPath $markerPath) -and (Test-Path -LiteralPath (Join-Path $template '.git'))) {
        try {
            $actual = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
            $markerMatches = $actual.control_version -eq $expectedMarker.control_version -and
                $actual.base_commit -eq $expectedMarker.base_commit -and
                $actual.setup_patch -eq $expectedMarker.setup_patch -and
                $actual.setup_patch_sha256 -eq $expectedMarker.setup_patch_sha256
        } catch { $markerMatches = $false }
    }
    if (-not $markerMatches) {
        $repo = Ensure-RepositoryCache -Case $Case
        Export-Revision -Repository $repo -Revision ([string]$Case.base_commit) -Destination $template -MutationRoot $templates
        Apply-SetupPatch -Case $Case -Root $template
        Prepare-CargoControls -Case $Case -Root $template
        Initialize-SyntheticRepository -Root $template -Case $Case
        Write-Utf8File -Path $markerPath -Content ($expectedMarker | ConvertTo-Json)
    }
    return $template
}

function Copy-Tree {
    param([string]$Source, [string]$Destination, [string]$MutationRoot)
    Reset-BenchmarkDirectory -Root $MutationRoot -Path $Destination
    Get-ChildItem -LiteralPath $Source -Force |
        Where-Object { $_.Name -notin @('target', '.pactrail') } |
        ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $Destination -Recurse -Force
    }
}

function Test-IgnoredRelativePath {
    param([string]$Relative)
    $first = ($Relative -replace '\\', '/') -split '/' | Select-Object -First 1
    return $first -in @('.git', '.pactrail', 'target')
}

function Get-VisibleSnapshot {
    param([string]$Root)
    $snapshot = [ordered]@{}
    if (-not (Test-Path -LiteralPath $Root)) { return $snapshot }
    foreach ($file in Get-ChildItem -LiteralPath $Root -Recurse -File -Force | Sort-Object FullName) {
        $relative = $file.FullName.Substring($Root.Length).TrimStart('\', '/') -replace '\\', '/'
        if (Test-IgnoredRelativePath -Relative $relative) { continue }
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

function Test-SnapshotEqual {
    param($Left, $Right)
    return @(Get-ChangedPaths -Before $Left -After $Right).Count -eq 0
}

function Copy-VisibleTree {
    param([string]$Source, [string]$Destination, [string]$MutationRoot)
    Reset-BenchmarkDirectory -Root $MutationRoot -Path $Destination
    foreach ($file in Get-ChildItem -LiteralPath $Source -Recurse -File -Force) {
        $relative = $file.FullName.Substring($Source.Length).TrimStart('\', '/') -replace '\\', '/'
        if (Test-IgnoredRelativePath -Relative $relative) { continue }
        $target = Join-Path $Destination ($relative -replace '/', [System.IO.Path]::DirectorySeparatorChar)
        $parent = Split-Path -Parent $target
        if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
        Copy-Item -LiteralPath $file.FullName -Destination $target -Force
    }
}

function Sync-SolutionIntoGrade {
    param([string]$Template, [string]$Solution, [string]$GradeRoot, [string]$MutationRoot)
    Copy-Tree -Source $Template -Destination $GradeRoot -MutationRoot $MutationRoot
    foreach ($child in Get-ChildItem -LiteralPath $GradeRoot -Force | Where-Object { $_.Name -ne '.git' }) {
        Assert-SafeChildPath -Root $MutationRoot -Path $child.FullName
        Remove-Item -LiteralPath $child.FullName -Recurse -Force
    }
    foreach ($file in Get-ChildItem -LiteralPath $Solution -Recurse -File -Force) {
        $relative = $file.FullName.Substring($Solution.Length).TrimStart('\', '/') -replace '\\', '/'
        if (Test-IgnoredRelativePath -Relative $relative) { continue }
        $target = Join-Path $GradeRoot ($relative -replace '/', [System.IO.Path]::DirectorySeparatorChar)
        $parent = Split-Path -Parent $target
        if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
        Copy-Item -LiteralPath $file.FullName -Destination $target -Force
    }
}

function Overlay-HiddenTests {
    param($Case, [string]$GradeRoot)
    $repo = Ensure-RepositoryCache -Case $Case
    foreach ($relative in $Case.hidden_overlay_paths) {
        $path = [string]$relative
        $show = Invoke-CapturedProcess -FileName 'git' -Arguments @('-C', $repo, 'show', "$($Case.reference_commit):$path") `
            -WorkingDirectory $repo -TimeoutSeconds 60 -StdoutPath '' -StderrPath ''
        if ($show.exit_code -ne 0) { throw "Could not load hidden test $path for $($Case.id)." }
        Write-Utf8File -Path (Join-Path $GradeRoot ($path -replace '/', [System.IO.Path]::DirectorySeparatorChar)) -Content $show.stdout
    }
}

function Get-PatchMetrics {
    param([string]$GradeRoot, [string]$PatchPath)
    [void](Invoke-Git -WorkingDirectory $GradeRoot -Arguments @('add', '--intent-to-add', '--all'))
    $diffResult = Invoke-CapturedProcess -FileName 'git' -Arguments @('diff', '--binary', '--no-ext-diff', 'HEAD', '--') `
        -WorkingDirectory $GradeRoot -TimeoutSeconds 60 -StdoutPath $PatchPath -StderrPath ''
    if ($diffResult.exit_code -ne 0) { throw "Could not render candidate patch: $($diffResult.stderr)" }
    $numstat = Invoke-Git -WorkingDirectory $GradeRoot -Arguments @('diff', '--numstat', 'HEAD', '--')
    $files = 0
    $additions = 0L
    $deletions = 0L
    foreach ($line in $numstat -split "`r?`n") {
        if (-not $line.Trim()) { continue }
        $parts = $line -split "`t"
        if ($parts.Count -lt 3) { continue }
        $files++
        if ($parts[0] -match '^\d+$') { $additions += [long]$parts[0] }
        if ($parts[1] -match '^\d+$') { $deletions += [long]$parts[1] }
    }
    return [pscustomobject]@{
        files = $files
        additions = $additions
        deletions = $deletions
        bytes = if (Test-Path -LiteralPath $PatchPath) { (Get-Item -LiteralPath $PatchPath).Length } else { 0 }
    }
}

function Invoke-GraderCommand {
    param($Command, [string]$GradeRoot, [string]$ArtifactDirectory, [string]$Name)
    $stdout = Join-Path $ArtifactDirectory "$Name-stdout.txt"
    $stderr = Join-Path $ArtifactDirectory "$Name-stderr.txt"
    $result = Invoke-CapturedProcess -FileName ([string]$Command.program) -Arguments @($Command.args | ForEach-Object { [string]$_ }) `
        -WorkingDirectory $GradeRoot -TimeoutSeconds ([int]$Command.timeout_seconds) -StdoutPath $stdout -StderrPath $stderr
    return [pscustomobject]@{
        name = $Name
        command = ([string]$Command.program + ' ' + (@($Command.args) -join ' '))
        exit_code = $result.exit_code
        timed_out = $result.timed_out
        duration_ms = $result.duration_ms
        passed = $result.exit_code -eq 0 -and -not $result.timed_out
    }
}

function Invoke-BehavioralGrade {
    param($Case, [string]$Template, [string]$Solution, [string]$GradeRoot, [string]$ArtifactDirectory, [string]$MutationRoot)
    Sync-SolutionIntoGrade -Template $Template -Solution $Solution -GradeRoot $GradeRoot -MutationRoot $MutationRoot
    $patch = Get-PatchMetrics -GradeRoot $GradeRoot -PatchPath (Join-Path $ArtifactDirectory 'candidate.patch')
    Overlay-HiddenTests -Case $Case -GradeRoot $GradeRoot
    $targeted = Invoke-GraderCommand -Command $Case.targeted_test -GradeRoot $GradeRoot -ArtifactDirectory $ArtifactDirectory -Name 'targeted-test'
    $regression = Invoke-GraderCommand -Command $Case.regression_test -GradeRoot $GradeRoot -ArtifactDirectory $ArtifactDirectory -Name 'regression-test'
    return [pscustomobject]@{
        passed = $targeted.passed -and $regression.passed
        targeted = $targeted
        regression = $regression
        patch = $patch
    }
}

function Get-PactrailTraceMetrics {
    param([string]$TracePath)
    $metrics = [ordered]@{
        events = 0; model_calls = 0; tool_calls = 0; failed_tool_calls = 0
        input_tokens = 0L; cached_input_tokens = 0L; output_tokens = 0L
        model_duration_ms = 0L; tool_duration_ms = 0L; recovery_stops = 0
    }
    if (-not (Test-Path -LiteralPath $TracePath)) { return [pscustomobject]$metrics }
    foreach ($line in Get-Content -LiteralPath $TracePath) {
        if (-not $line.Trim()) { continue }
        $event = $line | ConvertFrom-Json
        $metrics.events++
        if ($event.event.type -eq 'action_completed') {
            $data = $event.event.data
            if ($data.actor -like 'model:*') {
                $metrics.model_calls++
                $metrics.model_duration_ms += [long]$data.duration_ms
                if ($data.attributes.PSObject.Properties.Name -contains 'input_tokens') { $metrics.input_tokens += [long]$data.attributes.input_tokens }
                if ($data.attributes.PSObject.Properties.Name -contains 'cached_input_tokens') { $metrics.cached_input_tokens += [long]$data.attributes.cached_input_tokens }
                if ($data.attributes.PSObject.Properties.Name -contains 'output_tokens') { $metrics.output_tokens += [long]$data.attributes.output_tokens }
            } elseif ($data.actor -like 'tool:*') {
                $metrics.tool_calls++
                $metrics.tool_duration_ms += [long]$data.duration_ms
                if (-not $data.succeeded) { $metrics.failed_tool_calls++ }
            }
        } elseif ($event.event.type -eq 'note_recorded' -and $event.event.data.message -like '*recovery stopped*') {
            $metrics.recovery_stops++
        }
    }
    return [pscustomobject]$metrics
}

function Get-OpenCodeMetrics {
    param([string]$JsonLines)
    $metrics = [ordered]@{
        model_calls = 0; tool_calls = 0; failed_tool_calls = 0; errors = 0
        uncached_input_tokens = 0L; cached_input_tokens = 0L; output_tokens = 0L
        reasoning_tokens = 0L; summary = ''
    }
    $texts = New-Object System.Collections.Generic.List[string]
    foreach ($line in $JsonLines -split "`r?`n") {
        if (-not $line.Trim()) { continue }
        try { $event = $line | ConvertFrom-Json } catch { $metrics.errors++; continue }
        switch ($event.type) {
            'step_finish' {
                $metrics.model_calls++
                $metrics.uncached_input_tokens += [long]$event.part.tokens.input
                $metrics.cached_input_tokens += [long]$event.part.tokens.cache.read
                $metrics.output_tokens += [long]$event.part.tokens.output
                if ($event.part.tokens.PSObject.Properties.Name -contains 'reasoning') { $metrics.reasoning_tokens += [long]$event.part.tokens.reasoning }
            }
            'tool_use' {
                $metrics.tool_calls++
                if ($event.part.state.status -ne 'completed') { $metrics.failed_tool_calls++ }
            }
            'text' { [void]$texts.Add([string]$event.part.text) }
            'error' { $metrics.errors++ }
        }
    }
    $metrics.summary = $texts -join "`n"
    $object = [pscustomobject]$metrics
    $object | Add-Member -NotePropertyName input_tokens -NotePropertyValue ([long]($metrics.uncached_input_tokens + $metrics.cached_input_tokens))
    $object | Add-Member -NotePropertyName total_tokens -NotePropertyValue ([long]($metrics.uncached_input_tokens + $metrics.cached_input_tokens + $metrics.output_tokens))
    return $object
}

function Get-EstimatedCost {
    param([string]$ModelName, [long]$InputTokens, [long]$CachedTokens, [long]$OutputTokens)
    if ($ModelName -eq 'deepseek-v4-flash') {
        $missRate = 0.14; $hitRate = 0.0028; $outputRate = 0.28
    } else {
        $missRate = 0.435; $hitRate = 0.003625; $outputRate = 0.87
    }
    $miss = [Math]::Max(0L, $InputTokens - $CachedTokens)
    return [Math]::Round((($miss * $missRate) + ($CachedTokens * $hitRate) + ($OutputTokens * $outputRate)) / 1000000, 6)
}

function Test-ForbiddenChanges {
    param($Case, [string[]]$ChangedPaths)
    $violations = New-Object System.Collections.Generic.List[string]
    foreach ($changed in $ChangedPaths) {
        foreach ($forbidden in $Case.forbidden_paths) {
            $rule = ([string]$forbidden).TrimEnd('/')
            if ($changed -eq $rule -or $changed.StartsWith("$rule/", [StringComparison]::Ordinal)) {
                [void]$violations.Add($changed)
                break
            }
        }
    }
    return @($violations | Sort-Object -Unique)
}

function Invoke-PactrailRun {
    param($Case, [string]$Workspace, [string]$ArtifactDirectory, [string]$ApiKey)
    $command = Get-Command $Pactrail -ErrorAction Stop
    $arguments = @(
        'run', '--workspace', $Workspace,
        '--provider', 'open-ai-compatible', '--base-url', 'https://api.deepseek.com',
        '--model', $Model, '--api-key-env', $ApiKeyEnv,
        '--context-tokens', '16384', '--max-output-tokens', '1024',
        '--max-turns', '12', '--request-timeout-seconds', '600',
        '--disable-thinking', '--allow-process', '--write-path', '.', '--output', 'json',
        [string]$Case.prompt
    )
    return Invoke-CapturedProcess -FileName $command.Source -Arguments $arguments -WorkingDirectory $Workspace -TimeoutSeconds 600 `
        -StdoutPath (Join-Path $ArtifactDirectory 'run-output.json') -StderrPath (Join-Path $ArtifactDirectory 'run-stderr.txt') `
        -Environment @{ $ApiKeyEnv = $ApiKey }
}

function Invoke-OpenCodeRun {
    param($Case, [string]$Workspace, [string]$ArtifactDirectory, [string]$ApiKey, [string]$RuntimeRoot)
    $command = Get-Command $OpenCode -ErrorAction Stop
    $config = [System.IO.Path]::GetFullPath((Join-Path $suiteRoot 'opencode-deepseek.json'))
    $arguments = @('run', '--dir', $Workspace, '--model', "deepseek-direct/$Model", '--agent', 'build', '--format', 'json', [string]$Case.prompt)
    $fileName = $command.Source
    if ($command.CommandType -eq 'ExternalScript') {
        $fileName = (Get-Process -Id $PID).Path
        $arguments = @('-NoLogo', '-NoProfile', '-File', $command.Source) + $arguments
    }
    $environment = @{
        $ApiKeyEnv = $ApiKey
        'OPENCODE_CONFIG' = $config
        'OPENCODE_CONFIG_DIR' = (Join-Path $RuntimeRoot 'config-dir')
        'XDG_CONFIG_HOME' = (Join-Path $RuntimeRoot 'config')
        'XDG_DATA_HOME' = (Join-Path $RuntimeRoot 'data')
        'XDG_CACHE_HOME' = (Join-Path $RuntimeRoot 'cache')
    }
    New-Item -ItemType Directory -Force -Path $environment.OPENCODE_CONFIG_DIR | Out-Null
    return Invoke-CapturedProcess -FileName $fileName -Arguments $arguments -WorkingDirectory $Workspace -TimeoutSeconds 600 `
        -StdoutPath (Join-Path $ArtifactDirectory 'run-output.jsonl') -StderrPath (Join-Path $ArtifactDirectory 'run-stderr.txt') `
        -Environment $environment
}

function Validate-GradersNow {
    param([object[]]$SelectedCases)
    $validationRoot = Join-Path ([System.IO.Path]::GetFullPath($WorkspaceDirectory)) 'grader-validation'
    New-Item -ItemType Directory -Force -Path $validationRoot | Out-Null
    $records = New-Object System.Collections.ArrayList
    foreach ($case in $SelectedCases) {
        Write-Host "[grader] $($case.id)"
        $template = Ensure-Template -Case $case
        $caseRoot = Join-Path $validationRoot ([string]$case.id)
        Reset-BenchmarkDirectory -Root $validationRoot -Path $caseRoot

        $base = Join-Path $caseRoot 'base-with-hidden-tests'
        Copy-Tree -Source $template -Destination $base -MutationRoot $caseRoot
        Overlay-HiddenTests -Case $case -GradeRoot $base
        $baseResult = Invoke-GraderCommand -Command $case.targeted_test -GradeRoot $base -ArtifactDirectory $caseRoot -Name 'base-targeted'

        $reference = Join-Path $caseRoot 'reference'
        $repo = Ensure-RepositoryCache -Case $case
        Export-Revision -Repository $repo -Revision ([string]$case.reference_commit) -Destination $reference -MutationRoot $caseRoot
        Apply-SetupPatch -Case $case -Root $reference
        Prepare-CargoControls -Case $case -Root $reference
        $referenceTarget = Invoke-GraderCommand -Command $case.targeted_test -GradeRoot $reference -ArtifactDirectory $caseRoot -Name 'reference-targeted'
        $referenceRegression = Invoke-GraderCommand -Command $case.regression_test -GradeRoot $reference -ArtifactDirectory $caseRoot -Name 'reference-regression'
        $valid = (-not $baseResult.passed) -and $referenceTarget.passed -and $referenceRegression.passed
        [void]$records.Add([pscustomobject]@{
            case_id = [string]$case.id
            pre_fix_targeted_failed = -not $baseResult.passed
            reference_targeted_passed = $referenceTarget.passed
            reference_regression_passed = $referenceRegression.passed
            valid = $valid
        })
    }
    $output = [pscustomobject]@{
        schema_version = 1
        suite = [string]$manifest.suite
        validated_at = [DateTimeOffset]::UtcNow.ToString('o')
        passed = @($records | Where-Object { $_.valid }).Count
        total = $records.Count
        records = @($records)
    }
    $path = Join-Path ([System.IO.Path]::GetFullPath($OutputDirectory)) 'grader-validation.json'
    Write-Utf8File -Path $path -Content ($output | ConvertTo-Json -Depth 7)
    if ($output.passed -ne $output.total) { throw 'One or more behavioral graders failed gold validation.' }
    Write-Host "All $($output.total) graders reject the base revision and accept the reference revision."
}

$apiKey = Get-ApiKey
$balanceBefore = Get-DeepSeekBalance -ApiKey $apiKey
if ($balanceBefore -lt [decimal]$manifest.controls.minimum_balance_usd) {
    throw "DeepSeek balance $balanceBefore is below the preregistered floor $($manifest.controls.minimum_balance_usd)."
}

New-Item -ItemType Directory -Force -Path $OutputDirectory, $WorkspaceDirectory, $CacheDirectory | Out-Null
if ($ValidateGraders) {
    Validate-GradersNow -SelectedCases $cases
    return
}

$runStamp = [DateTimeOffset]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$modelSlug = $Model -replace '[^A-Za-z0-9._-]', '-'
$resultRoot = Join-Path ([System.IO.Path]::GetFullPath($OutputDirectory)) "scored/$Harness/$modelSlug/$runStamp"
$matrixWorkspace = Join-Path ([System.IO.Path]::GetFullPath($WorkspaceDirectory)) "$runStamp-$Harness-$modelSlug"
New-Item -ItemType Directory -Force -Path $resultRoot, $matrixWorkspace | Out-Null

$pactrailVersion = if ($Harness -eq 'pactrail') { (& $Pactrail --version 2>&1 | Out-String).Trim() } else { $null }
$openCodeVersion = if ($Harness -eq 'opencode') { (& $OpenCode --version 2>&1 | Out-String).Trim() } else { $null }
$results = New-Object System.Collections.ArrayList

foreach ($case in $cases) {
    $currentBalance = Get-DeepSeekBalance -ApiKey $apiKey
    if ($currentBalance -lt [decimal]$manifest.controls.minimum_balance_usd) {
        throw "Budget floor reached before $($case.id); refusing additional model calls."
    }
    Write-Host "[$Harness/$Model] $($case.id)"
    $caseRoot = Join-Path $matrixWorkspace ([string]$case.id)
    $workspace = Join-Path $caseRoot 'workspace'
    $gradeRoot = Join-Path $caseRoot 'grade'
    $runtimeRoot = Join-Path $caseRoot 'opencode-runtime'
    $artifact = Join-Path $resultRoot ([string]$case.id)
    Reset-BenchmarkDirectory -Root $matrixWorkspace -Path $caseRoot
    New-Item -ItemType Directory -Force -Path $artifact | Out-Null
    $template = Ensure-Template -Case $case
    Copy-Tree -Source $template -Destination $workspace -MutationRoot $caseRoot
    $baseline = Get-VisibleSnapshot -Root $workspace

    if ($Harness -eq 'pactrail') {
        $invoke = Invoke-PactrailRun -Case $case -Workspace $workspace -ArtifactDirectory $artifact -ApiKey $apiKey
        try { $runJson = $invoke.stdout | ConvertFrom-Json } catch { $runJson = $null }
        $runId = if ($null -ne $runJson) { [string]$runJson.run_id } else { '' }
        if (-not $runId -and $invoke.stderr -match 'run ([0-9a-f-]{36})') { $runId = $Matches[1] }
        $runDirectory = if ($runId) { Join-Path $workspace ".pactrail/runs/$runId" } else { '' }
        $candidate = if ($runDirectory) { Join-Path $runDirectory 'workspace' } else { '' }
        $solution = if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Container)) { $candidate } else { $workspace }
        $sourceBeforeApply = Get-VisibleSnapshot -Root $workspace
        $isolated = Test-SnapshotEqual -Left $baseline -Right $sourceBeforeApply
        $solutionSnapshotForResult = Get-VisibleSnapshot -Root $solution
        Copy-VisibleTree -Source $solution -Destination (Join-Path $artifact 'solution-workspace') -MutationRoot $artifact
        $grade = Invoke-BehavioralGrade -Case $case -Template $template -Solution $solution -GradeRoot $gradeRoot -ArtifactDirectory $artifact -MutationRoot $caseRoot

        $traceValid = $false
        $tracePath = if ($runDirectory) { Join-Path $runDirectory 'trace.jsonl' } else { '' }
        if ($runId) {
            $trace = Invoke-CapturedProcess -FileName (Get-Command $Pactrail).Source -Arguments @('trace', '--workspace', $workspace, '--json', $runId) `
                -WorkingDirectory $workspace -TimeoutSeconds 60 -StdoutPath (Join-Path $artifact 'trace-render.json') -StderrPath (Join-Path $artifact 'trace-stderr.txt')
            $traceValid = $trace.exit_code -eq 0
            if (Test-Path -LiteralPath $tracePath) { Copy-Item -LiteralPath $tracePath -Destination (Join-Path $artifact 'trace.jsonl') }
            $receiptPath = Join-Path $runDirectory 'receipt.json'
            if (Test-Path -LiteralPath $receiptPath) { Copy-Item -LiteralPath $receiptPath -Destination (Join-Path $artifact 'receipt.json') }
        }
        $applyExit = $null
        if ($null -ne $runJson -and $runJson.outcome -eq 'ready_to_apply' -and $runId) {
            $apply = Invoke-CapturedProcess -FileName (Get-Command $Pactrail).Source -Arguments @('apply', '--workspace', $workspace, '--json', $runId) `
                -WorkingDirectory $workspace -TimeoutSeconds 120 -StdoutPath (Join-Path $artifact 'apply-output.json') -StderrPath (Join-Path $artifact 'apply-stderr.txt')
            $applyExit = $apply.exit_code
        }
        $appliedSnapshot = Get-VisibleSnapshot -Root $workspace
        $appliedMatches = $null -ne $applyExit -and $applyExit -eq 0 -and (Test-SnapshotEqual -Left $solutionSnapshotForResult -Right $appliedSnapshot)
        $metrics = Get-PactrailTraceMetrics -TracePath $tracePath
        $inputTokens = [long]$metrics.input_tokens
        $cachedTokens = [long]$metrics.cached_input_tokens
        $outputTokens = [long]$metrics.output_tokens
        $providerErrors = if ($invoke.exit_code -eq 0) { 0 } else { 1 }
        $summaryText = if ($null -ne $runJson) { [string]$runJson.summary } else { '' }
        $outcome = if ($null -ne $runJson) { [string]$runJson.outcome } else { 'process_error' }
    } else {
        $invoke = Invoke-OpenCodeRun -Case $case -Workspace $workspace -ArtifactDirectory $artifact -ApiKey $apiKey -RuntimeRoot $runtimeRoot
        $solution = $workspace
        $solutionSnapshotForResult = Get-VisibleSnapshot -Root $solution
        Copy-VisibleTree -Source $solution -Destination (Join-Path $artifact 'solution-workspace') -MutationRoot $artifact
        $grade = Invoke-BehavioralGrade -Case $case -Template $template -Solution $solution -GradeRoot $gradeRoot -ArtifactDirectory $artifact -MutationRoot $caseRoot
        $metrics = Get-OpenCodeMetrics -JsonLines $invoke.stdout
        $inputTokens = [long]$metrics.input_tokens
        $cachedTokens = [long]$metrics.cached_input_tokens
        $outputTokens = [long]$metrics.output_tokens
        $providerErrors = [int]$metrics.errors
        $summaryText = [string]$metrics.summary
        $outcome = if ($invoke.timed_out) { 'timeout' } elseif ($invoke.exit_code -eq 0) { 'completed' } else { 'process_error' }
        $isolated = $false
        $traceValid = $false
        $applyExit = $null
        $appliedMatches = $false
        $runId = ''
    }

    $changedPaths = @(Get-ChangedPaths -Before $baseline -After $solutionSnapshotForResult)
    $forbidden = @(Test-ForbiddenChanges -Case $case -ChangedPaths $changedPaths)
    $instructionsPassed = $forbidden.Count -eq 0
    $functionalPassed = [bool]$grade.passed
    $taskPassed = $functionalPassed -and $instructionsPassed
    $strictPassed = $taskPassed -and -not $invoke.timed_out -and $providerErrors -eq 0
    if ($Harness -eq 'pactrail') {
        $strictPassed = $strictPassed -and $outcome -eq 'ready_to_apply' -and $isolated -and $traceValid -and $appliedMatches
    }
    $cost = Get-EstimatedCost -ModelName $Model -InputTokens $inputTokens -CachedTokens $cachedTokens -OutputTokens $outputTokens
    $record = [pscustomobject]@{
        schema_version = 1
        suite = [string]$manifest.suite
        case_id = [string]$case.id
        repository = [string]$case.repository
        difficulty = [string]$case.difficulty
        harness = $Harness
        harness_version = if ($Harness -eq 'pactrail') { $pactrailVersion } else { $openCodeVersion }
        model = $Model
        functional_passed = $functionalPassed
        instructions_passed = $instructionsPassed
        task_passed = $taskPassed
        strict_passed = $strictPassed
        outcome = $outcome
        exit_code = $invoke.exit_code
        timed_out = $invoke.timed_out
        duration_ms = $invoke.duration_ms
        grading_duration_ms = [long]($grade.targeted.duration_ms + $grade.regression.duration_ms)
        run_id = $runId
        changed_paths = $changedPaths
        forbidden_changes = $forbidden
        patch = $grade.patch
        targeted_test = $grade.targeted
        regression_test = $grade.regression
        input_tokens = $inputTokens
        cached_input_tokens = $cachedTokens
        output_tokens = $outputTokens
        total_tokens = [long]($inputTokens + $outputTokens)
        estimated_cost_usd = $cost
        model_calls = [int]$metrics.model_calls
        tool_calls = [int]$metrics.tool_calls
        failed_tool_calls = [int]$metrics.failed_tool_calls
        provider_errors = $providerErrors
        source_isolation_preserved = $isolated
        trace_integrity_verified = $traceValid
        applied_candidate_matches = $appliedMatches
        summary = $summaryText
    }
    [void]$results.Add($record)
    Write-Utf8File -Path (Join-Path $artifact 'result.json') -Content ($record | ConvertTo-Json -Depth 10)
    if (-not $KeepWorkspaces) {
        Assert-SafeChildPath -Root $matrixWorkspace -Path $caseRoot
        Remove-Item -LiteralPath $caseRoot -Recurse -Force
    }
}

$balanceAfter = Get-DeepSeekBalance -ApiKey $apiKey
$durations = @($results | ForEach-Object { [long]$_.duration_ms } | Sort-Object)
$median = if ($durations.Count % 2 -eq 0) {
    [long](($durations[$durations.Count / 2 - 1] + $durations[$durations.Count / 2]) / 2)
} else {
    $durations[[int][Math]::Floor($durations.Count / 2)]
}
$summary = [pscustomobject]@{
    schema_version = 1
    suite = [string]$manifest.suite
    harness = $Harness
    harness_version = if ($Harness -eq 'pactrail') { $pactrailVersion } else { $openCodeVersion }
    model = $Model
    started_at = $runStamp
    cases = $results.Count
    functional_passed = @($results | Where-Object { $_.functional_passed }).Count
    task_passed = @($results | Where-Object { $_.task_passed }).Count
    strict_passed = @($results | Where-Object { $_.strict_passed }).Count
    median_duration_ms = $median
    total_duration_ms = [long](($results | Measure-Object -Property duration_ms -Sum).Sum)
    total_input_tokens = [long](($results | Measure-Object -Property input_tokens -Sum).Sum)
    total_cached_input_tokens = [long](($results | Measure-Object -Property cached_input_tokens -Sum).Sum)
    total_output_tokens = [long](($results | Measure-Object -Property output_tokens -Sum).Sum)
    total_tokens = [long](($results | Measure-Object -Property total_tokens -Sum).Sum)
    estimated_cost_usd = [Math]::Round([double](($results | Measure-Object -Property estimated_cost_usd -Sum).Sum), 6)
    balance_before_usd = $balanceBefore.ToString('0.00')
    balance_after_usd = $balanceAfter.ToString('0.00')
    results = @($results)
}
Write-Utf8File -Path (Join-Path $resultRoot 'summary.json') -Content ($summary | ConvertTo-Json -Depth 12)

$markdown = New-Object System.Collections.Generic.List[string]
$markdown.Add("# $Harness / $Model")
$markdown.Add('')
$markdown.Add("- Strict task result: **$($summary.strict_passed)/$($summary.cases)**")
$markdown.Add("- Behavioral tests: **$($summary.functional_passed)/$($summary.cases)**")
$markdown.Add("- Provider-reported tokens: **$($summary.total_tokens)**")
$markdown.Add("- Median agent wall time: **$([Math]::Round($summary.median_duration_ms / 1000, 2)) s**")
$markdown.Add("- Estimated API cost: **`$$($summary.estimated_cost_usd)`**")
$markdown.Add('')
$markdown.Add('| Task | Strict | Hidden tests | Time | Tokens | Patch |')
$markdown.Add('|---|---:|---:|---:|---:|---:|')
foreach ($result in $results) {
    $markdown.Add("| $($result.case_id) | $(if ($result.strict_passed) { 'PASS' } else { 'FAIL' }) | $(if ($result.functional_passed) { 'PASS' } else { 'FAIL' }) | $([Math]::Round($result.duration_ms / 1000, 2)) s | $($result.total_tokens) | +$($result.patch.additions)/-$($result.patch.deletions) |")
}
Write-Utf8File -Path (Join-Path $resultRoot 'SUMMARY.md') -Content ($markdown -join "`n")

Write-Host ''
Write-Host "$($summary.strict_passed)/$($summary.cases) strict tasks passed. Results: $resultRoot"
if ($summary.strict_passed -ne $summary.cases) { exit 2 }
