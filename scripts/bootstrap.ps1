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

function Invoke-Bootstrap {
    param([string[]]$BootstrapArguments)

    & cargo run -p bootstrap -- @BootstrapArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Rust bootstrap failed with exit code $LASTEXITCODE."
    }
}

Push-Location $repoRoot
try {
    Invoke-Bootstrap @('init', '--non-interactive', '--yes')
    if ($Verify) {
        Invoke-Bootstrap @('verify')
    }

    $buildArguments = @('build')
    if ($Release) {
        $buildArguments += '--release'
    }
    Invoke-Bootstrap $buildArguments

    if ($NoStart) {
        return
    }

    $serverArguments = @('server', '--listen', $Listen)
    if ($Release) {
        $serverArguments += '--release'
    }
    $hostName = $Listen.Substring(0, $Listen.LastIndexOf(':')).Trim('[', ']')
    $ipAddress = $null
    if ([Net.IPAddress]::TryParse($hostName, [ref]$ipAddress) -and -not [Net.IPAddress]::IsLoopback($ipAddress)) {
        $serverArguments += '--allow-public-bind'
    }
    Invoke-Bootstrap $serverArguments
}
finally {
    Pop-Location
}
