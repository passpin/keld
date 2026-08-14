[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repositoryRoot = Split-Path -Parent $scriptRoot
$lock = Get-Content -Raw -LiteralPath (Join-Path $repositoryRoot '.tools\llvm\llvm-packages.lock.json') | ConvertFrom-Json
$prefix = Join-Path (Join-Path $repositoryRoot '.tools\llvm') $lock.prefix_directory

if (-not (Test-Path -LiteralPath (Join-Path $prefix 'bin\llvm-config.exe'))) {
    $prefix = & (Join-Path $scriptRoot 'bootstrap-llvm.ps1')
}

$env:LLVM_SYS_221_PREFIX = $prefix
$env:PATH = "$(Join-Path $prefix 'bin');$env:PATH"
Write-Output "LLVM_SYS_221_PREFIX=$env:LLVM_SYS_221_PREFIX"
