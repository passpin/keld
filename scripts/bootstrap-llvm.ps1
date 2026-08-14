[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repositoryRoot = Split-Path -Parent $scriptRoot
$lockPath = Join-Path $repositoryRoot '.tools\llvm\llvm-packages.lock.json'
$toolRoot = Join-Path $repositoryRoot '.tools\llvm'
$lock = Get-Content -Raw -LiteralPath $lockPath | ConvertFrom-Json
$prefix = Join-Path $toolRoot $lock.prefix_directory

function Assert-LockedString([object]$value, [string]$name) {
    if ([string]::IsNullOrWhiteSpace([string]$value)) {
        throw "LLVM lock entry '$name' is empty."
    }
}

Assert-LockedString $lock.llvm_version 'llvm_version'
Assert-LockedString $lock.llvm_sys_version 'llvm_sys_version'
Assert-LockedString $lock.target 'target'
if ($lock.llvm_version -ne '22.1.8' -or $lock.llvm_sys_version -ne '221.0.1') {
    throw 'The LLVM lock is not the frozen Native-1 version.'
}
if ($lock.target -ne 'x86_64-w64-windows-gnu') {
    throw "The LLVM lock targets '$($lock.target)', not x86_64-w64-windows-gnu."
}

New-Item -ItemType Directory -Force -Path $toolRoot | Out-Null

if (Test-Path -LiteralPath $prefix) {
    $existingConfig = Join-Path $prefix 'bin\llvm-config.exe'
    if (-not (Test-Path -LiteralPath $existingConfig)) {
        throw "Existing LLVM prefix is incomplete: $prefix"
    }
    $existingVersion = (& $existingConfig --version).Trim()
    if ($existingVersion -ne $lock.llvm_version) {
        throw "Existing LLVM prefix reports '$existingVersion', expected '$($lock.llvm_version)'."
    }
    Write-Output $prefix
    exit 0
}

$runId = [guid]::NewGuid().ToString('N')
$downloadRoot = Join-Path ([IO.Path]::GetTempPath()) "keld-llvm-download-$runId"
$stageRoot = Join-Path $toolRoot ".stage-$runId"
New-Item -ItemType Directory -Force -Path $downloadRoot, $stageRoot | Out-Null

try {
    $tar = Get-Command tar.exe -ErrorAction SilentlyContinue
    if ($null -eq $tar) {
        throw 'tar.exe is required to extract the pinned .pkg.tar.zst archives.'
    }

    foreach ($package in $lock.packages) {
        Assert-LockedString $package.name 'package.name'
        Assert-LockedString $package.url 'package.url'
        Assert-LockedString $package.sha256 'package.sha256'
        $archiveName = Split-Path -Leaf ([Uri]$package.url)
        $archivePath = Join-Path $downloadRoot $archiveName
        Invoke-WebRequest -UseBasicParsing -Uri $package.url -OutFile $archivePath
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
        $expected = ([string]$package.sha256).ToLowerInvariant()
        if ($actual -ne $expected) {
            throw "Checksum mismatch for ${archiveName}: expected $expected, got $actual"
        }
        & $tar.Source -xf $archivePath -C $stageRoot
        if ($LASTEXITCODE -ne 0) {
            throw "Unable to extract $archiveName (tar exit $LASTEXITCODE)."
        }
    }

    $candidate = Join-Path $stageRoot 'mingw64'
    $llvmConfig = Join-Path $candidate 'bin\llvm-config.exe'
    $llvmDll = Join-Path $candidate 'bin\libLLVM-22.dll'
    if (-not (Test-Path -LiteralPath $llvmConfig) -or -not (Test-Path -LiteralPath $llvmDll)) {
        throw 'The extracted package closure is missing llvm-config.exe or libLLVM-22.dll.'
    }
    $env:PATH = "$(Split-Path -Parent $llvmConfig);$env:PATH"
    $version = (& $llvmConfig --version).Trim()
    if ($version -ne $lock.llvm_version) {
        throw "Bootstrapped llvm-config reports '$version', expected '$($lock.llvm_version)'."
    }

    Move-Item -LiteralPath $candidate -Destination $prefix
    Write-Output $prefix
}
finally {
    if (Test-Path -LiteralPath $downloadRoot) {
        Remove-Item -LiteralPath $downloadRoot -Recurse -Force
    }
    if (Test-Path -LiteralPath $stageRoot) {
        Remove-Item -LiteralPath $stageRoot -Recurse -Force
    }
}
