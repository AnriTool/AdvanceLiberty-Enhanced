$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path $PSScriptRoot -Parent

function Get-GameRom([string]$Rom) {
    if (!$Rom) { $Rom = Join-Path $ProjectRoot 'roms/usa.gba' }
    if (!(Test-Path -LiteralPath $Rom -PathType Leaf)) {
        throw "USA ROM not found: $Rom. Place your ROM in roms/usa.gba or use -Rom PATH."
    }
    (Resolve-Path -LiteralPath $Rom).Path
}

function Get-NativeLibraries {
    $manifest = Get-Content -LiteralPath (Join-Path $ProjectRoot 'native/usa/manifest.json') -Raw | ConvertFrom-Json
    @($manifest.libraries | ForEach-Object {
        $path = Join-Path $ProjectRoot $_.path
        if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw "Native library missing: $path" }
        (Resolve-Path -LiteralPath $path).Path
    })
}

function Get-GameExecutable([switch]$SkipBuild) {
    $exe = Join-Path $ProjectRoot 'target/release/gta-advance.exe'
    if (!$SkipBuild -or !(Test-Path -LiteralPath $exe)) {
        & (Join-Path $ProjectRoot 'build.ps1') | Out-Host
    }
    $exe
}
