"""Exercise actual installers against local release assets; no network or user install."""
import hashlib
import os
from pathlib import Path
import platform
import subprocess
import sys
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(sys.argv[1]).resolve()
VERSION = subprocess.check_output([str(BINARY), '--version'], text=True).strip().split()[1]
with tempfile.TemporaryDirectory(prefix='pactrail-installer-') as temporary:
    root = Path(temporary)
    fixture = root / 'assets'
    fixture.mkdir()
    windows = sys.platform == 'win32'
    asset = 'pactrail-windows-x86_64.zip' if windows else ('pactrail-macos-aarch64.tar.gz' if platform.system() == 'Darwin' else 'pactrail-linux-x86_64.tar.gz')
    files = [(BINARY, 'pactrail.exe' if windows else 'pactrail')] + [(ROOT / name, name) for name in ['README.md', 'LICENSE', 'THIRD_PARTY_NOTICES']]
    if windows:
        with zipfile.ZipFile(fixture / asset, 'w') as archive:
            for source, name in files:
                archive.write(source, name)
    else:
        with tarfile.open(fixture / asset, 'w:gz') as archive:
            for source, name in files:
                archive.add(source, arcname=name)
    checksum = hashlib.sha256((fixture / asset).read_bytes()).hexdigest()
    manifest = fixture / 'SHA256SUMS'
    manifest.write_text(f'{checksum}  artifacts/{asset}\n')
    destination = root / 'installed'
    env = dict(os.environ, PACTRAIL_INSTALL_FIXTURE=str(fixture), PACTRAIL_INSTALL_DIR=str(destination), PACTRAIL_VERSION='v' + VERSION)
    if windows:
        wrapper = root / 'exercise.ps1'
        wrapper.write_text('''param([string]$Installer,[string]$Destination,[string]$Version,[switch]$ExpectFailure)
$ErrorActionPreference = "Stop"
function Invoke-WebRequest {
  param([string]$Uri,[string]$OutFile,[switch]$UseBasicParsing)
  Copy-Item -LiteralPath (Join-Path $env:PACTRAIL_INSTALL_FIXTURE ([Uri]$Uri).Segments[-1]) -Destination $OutFile
}
try {
  & $Installer -InstallDir $Destination -Version $Version -NoModifyPath
  if ($ExpectFailure) { throw "installer unexpectedly accepted a bad checksum" }
} catch {
  if (!$ExpectFailure -or $_.Exception.Message -notmatch "SHA-256 verification failed") { throw }
}
''')
        command = ['powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(wrapper), '-Installer', str(ROOT / 'install.ps1'), '-Destination', str(destination), '-Version', 'v' + VERSION]
    else:
        commands = root / 'commands'
        commands.mkdir()
        curl = commands / 'curl'
        curl.write_text('''#!/usr/bin/env python3
import os, pathlib, shutil, sys
arguments = sys.argv[1:]
url = next(arg for arg in arguments if arg.startswith('https://'))
destination = arguments[arguments.index('--output') + 1]
shutil.copyfile(pathlib.Path(os.environ['PACTRAIL_INSTALL_FIXTURE']) / url.rsplit('/', 1)[-1], destination)
''')
        curl.chmod(0o755)
        env['PATH'] = str(commands) + os.pathsep + env['PATH']
        command = ['sh', str(ROOT / 'install.sh')]
    subprocess.run(command, env=env, check=True)
    installed = destination / ('pactrail.exe' if windows else 'pactrail')
    assert subprocess.check_output([str(installed), '--version'], text=True).strip() == 'pactrail ' + VERSION
    before = hashlib.sha256(installed.read_bytes()).hexdigest()
    manifest.write_text(f'{"0" * 64}  artifacts/{asset}\n')
    bad = subprocess.run(command + (['-ExpectFailure'] if windows else []), env=env, capture_output=True, text=True)
    assert bad.returncode == (0 if windows else 1), bad.stdout + bad.stderr
    assert hashlib.sha256(installed.read_bytes()).hexdigest() == before, 'failed install changed existing binary'
    print('Installer accepted verified asset and rejected tampering without changing the existing installation.')
