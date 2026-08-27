[CmdletBinding()]
param(
    [ValidatePattern('^[^\s]+:\d+$')]
    [string]$Listen = '127.0.0.1:3000',
    [switch]$Release,
    [switch]$Verify,
    [switch]$NoStart
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$webRoot = Join-Path $repoRoot 'apps\oreak-web'
$webDist = Join-Path $webRoot 'dist'
$trunkConfig = Join-Path $webRoot 'Trunk.toml'

function Require-Command {
    param([string]$Name)

    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        throw "Required command '$Name' was not found on PATH."
    }
    return $command.Source
}

function Invoke-Checked {
    param(
        [string]$Executable,
        [string[]]$Arguments
    )

    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $Executable $($Arguments -join ' ')"
    }
}

function Assert-RustVersion {
    param([string]$Rustc)

    $versionOutput = & $Rustc --version
    if ($LASTEXITCODE -ne 0 -or $versionOutput -notmatch 'rustc\s+(\d+)\.(\d+)') {
        throw "Unable to determine the installed Rust version."
    }
    $major = [int]$Matches[1]
    $minor = [int]$Matches[2]
    if ($major -lt 1 -or ($major -eq 1 -and $minor -lt 85)) {
        throw "Oreak requires Rust 1.85 or newer; found '$versionOutput'."
    }
}

function Get-Trunk {
    $command = Get-Command trunk -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }

    Write-Host 'Installing Trunk 0.21.14...'
    Invoke-Checked (Require-Command 'cargo') @('install', '--locked', 'trunk@0.21.14')

    $cargoBin = Join-Path $HOME '.cargo\bin'
    $candidate = Join-Path $cargoBin 'trunk.exe'
    if (Test-Path -LiteralPath $candidate) {
        return $candidate
    }
    $command = Get-Command trunk -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }
    throw "Trunk was installed but could not be located. Add '$cargoBin' to PATH and run again."
}

if (-not (Test-Path -LiteralPath $webRoot) -or -not (Test-Path -LiteralPath $trunkConfig)) {
    throw "Expected Oreak web application files under '$webRoot'."
}

$cargo = Require-Command 'cargo'
$rustc = Require-Command 'rustc'
$rustup = Require-Command 'rustup'
Assert-RustVersion $rustc

$installedTargets = & $rustup target list --installed
if ($LASTEXITCODE -ne 0) {
    throw 'Unable to list installed Rust targets.'
}
if ($installedTargets -notcontains 'wasm32-unknown-unknown') {
    Write-Host 'Installing wasm32-unknown-unknown target...'
    Invoke-Checked $rustup @('target', 'add', 'wasm32-unknown-unknown')
}

$trunk = Get-Trunk

Push-Location $repoRoot
try {
    if ($Verify) {
        Write-Host 'Running workspace verification...'
        Invoke-Checked $cargo @('fmt', '--all', '--check')
        Invoke-Checked $cargo @('check', '--workspace', '--all-targets')
        Invoke-Checked $cargo @('test', '--workspace')
        Invoke-Checked $cargo @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings')
    }

    Write-Host 'Building server...'
    $cargoBuildArgs = @('build', '-p', 'oreak-server')
    if ($Release) {
        $cargoBuildArgs += '--release'
    }
    Invoke-Checked $cargo $cargoBuildArgs

    Write-Host 'Building browser application...'
    Push-Location $webRoot
    try {
        $trunkArgs = @('build', '--config', $trunkConfig)
        if ($Release) {
            $trunkArgs += '--release'
        }
        Invoke-Checked $trunk $trunkArgs
    }
    finally {
        Pop-Location
    }

    if ($NoStart) {
        Write-Host 'Bootstrap completed; server was not started because -NoStart was supplied.'
        return
    }

    $env:OREAK_LISTEN = $Listen
    $env:OREAK_WEB_DIST = $webDist
    if ([string]::IsNullOrWhiteSpace($env:RUST_LOG)) {
        $env:RUST_LOG = 'info'
    }

    Write-Host "Starting Oreak development server at http://$Listen"
    Write-Host 'Press Ctrl+C to stop the server. All MVP data is process-local.'
    $cargoRunArgs = @('run', '-p', 'oreak-server')
    if ($Release) {
        $cargoRunArgs += '--release'
    }
    Invoke-Checked $cargo $cargoRunArgs
}
finally {
    Pop-Location
}
