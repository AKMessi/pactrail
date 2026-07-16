[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\Pactrail\bin"),
    [string]$Version = $env:PACTRAIL_VERSION,
    [switch]$NoModifyPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = "latest"
}
if ($Version -ne "latest" -and $Version -notmatch '^v[0-9]+(?:\.[0-9]+){2}(?:-[0-9A-Za-z.-]+)?$') {
    throw "PACTRAIL_VERSION must be 'latest' or a semantic v-prefixed release tag."
}
if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    throw "InstallDir cannot be empty."
}

$Architecture = $env:PROCESSOR_ARCHITEW6432
if ([string]::IsNullOrWhiteSpace($Architecture)) {
    $Architecture = $env:PROCESSOR_ARCHITECTURE
}
if ($Architecture -notin @("AMD64", "ARM64")) {
    throw "No prebuilt Pactrail binary is available for Windows architecture '$Architecture'. Use the Cargo install command from the README."
}

$Repository = "AKMessi/pactrail"
$Asset = "pactrail-windows-x86_64.zip"
if ($Version -eq "latest") {
    $ReleaseBase = "https://github.com/$Repository/releases/latest/download"
}
else {
    $ReleaseBase = "https://github.com/$Repository/releases/download/$Version"
}

$TemporaryDir = Join-Path ([IO.Path]::GetTempPath()) ("pactrail-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $TemporaryDir | Out-Null
try {
    $Archive = Join-Path $TemporaryDir $Asset
    $Checksums = Join-Path $TemporaryDir "SHA256SUMS"
    Write-Host "Downloading Pactrail $Version for Windows/$Architecture..."
    Invoke-WebRequest -Uri "$ReleaseBase/$Asset" -OutFile $Archive -UseBasicParsing
    Invoke-WebRequest -Uri "$ReleaseBase/SHA256SUMS" -OutFile $Checksums -UseBasicParsing

    $AssetPattern = [Regex]::Escape($Asset)
    $Manifest = Get-Content -LiteralPath $Checksums -Raw
    $Match = [Regex]::Match(
        $Manifest,
        "(?im)^([0-9a-f]{64})\s+\*?(?:artifacts/)?${AssetPattern}\s*$"
    )
    if (-not $Match.Success) {
        throw "The release checksum manifest has no valid entry for $Asset."
    }
    $Expected = $Match.Groups[1].Value.ToLowerInvariant()
    $Actual = (Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "SHA-256 verification failed for $Asset."
    }

    $UnpackDir = Join-Path $TemporaryDir "unpack"
    Expand-Archive -LiteralPath $Archive -DestinationPath $UnpackDir
    $Binary = Get-ChildItem -LiteralPath $UnpackDir -Recurse -Filter "pactrail.exe" -File |
        Select-Object -First 1
    if ($null -eq $Binary) {
        throw "The release archive does not contain pactrail.exe."
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $Destination = Join-Path $InstallDir "pactrail.exe"
    $Running = Get-Process -Name "pactrail" -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and $_.Path.Equals($Destination, [StringComparison]::OrdinalIgnoreCase) }
    if ($Running) {
        throw "Close the running Pactrail process installed at '$Destination', then run the installer again."
    }
    $Staged = "$Destination.new"
    Copy-Item -LiteralPath $Binary.FullName -Destination $Staged -Force
    Move-Item -LiteralPath $Staged -Destination $Destination -Force

    $AddedToUserPath = $false
    if (-not $NoModifyPath) {
        $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
        $UserEntries = @($UserPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        $OnUserPath = $UserEntries | Where-Object {
            $_.TrimEnd('\').Equals($InstallDir.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)
        }
        if (-not $OnUserPath) {
            if ([string]::IsNullOrWhiteSpace($UserPath)) {
                $NewUserPath = $InstallDir
            }
            else {
                $NewUserPath = $InstallDir + ';' + $UserPath.TrimStart(';')
            }
            [Environment]::SetEnvironmentVariable("Path", $NewUserPath, "User")
            $AddedToUserPath = $true
        }
        $ProcessEntries = @($env:Path -split ';')
        $OnProcessPath = $ProcessEntries | Where-Object {
            $_.TrimEnd('\').Equals($InstallDir.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)
        }
        if (-not $OnProcessPath) {
            $env:Path = $InstallDir + ';' + $env:Path.TrimStart(';')
        }
    }

    $InstalledVersion = & $Destination --version
    Write-Host "Installed $InstalledVersion"
    if ($AddedToUserPath) {
        Write-Host "Added $InstallDir to your user PATH. New terminals can run pactrail directly."
    }
}
finally {
    if (Test-Path -LiteralPath $TemporaryDir) {
        Remove-Item -LiteralPath $TemporaryDir -Recurse -Force
    }
}
