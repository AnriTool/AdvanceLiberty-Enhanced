param(
    [Alias('Profile')][ValidateSet('precise', 'extra', 'mission', 'sessions', 'all')][string]$NativeProfile = 'all',
    [string]$Rom,
    [string]$Gcc,
    [string]$OutputDirectory,
    [switch]$SkipBuild,
    [switch]$Install
)
. (Join-Path $PSScriptRoot 'Project.ps1')
$Rom = Get-GameRom $Rom
$manifestPath = Join-Path $ProjectRoot 'native/usa/manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ((Get-FileHash -LiteralPath $Rom).Hash.ToLowerInvariant() -ne $manifest.rom_sha256) { throw 'Native profiles require the verified USA ROM.' }
if (!$Gcc) {
    $command = Get-Command gcc -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath 'C:/msys64/mingw64/bin/gcc.exe') { $Gcc = 'C:/msys64/mingw64/bin/gcc.exe' }
    elseif ($command) { $Gcc = $command.Source }
    else { throw 'MinGW GCC required. Pass -Gcc PATH.' }
}
$Gcc = (Resolve-Path -LiteralPath $Gcc).Path
$gccTarget = & $Gcc -dumpmachine
if ($LASTEXITCODE -ne 0 -or $gccTarget -notmatch '^x86_64-w64-mingw32$') { throw 'Use MinGW x64 GCC (x86_64-w64-mingw32), not MSYS GCC.' }
if (!$SkipBuild) { & (Join-Path $ProjectRoot 'build.ps1') -Tools | Out-Host }
$compiler = Join-Path $ProjectRoot 'target/release/advance-aot-supplement.exe'
if (!(Test-Path -LiteralPath $compiler)) { throw 'Build the AOT tool first.' }
if (!$OutputDirectory) { $OutputDirectory = Join-Path $ProjectRoot ('artifacts/native-build-' + [Guid]::NewGuid().ToString('N')) }
$OutputDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Native output must be a new directory.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
Copy-Item -LiteralPath $compiler -Destination (Join-Path $OutputDirectory 'advance-aot-supplement.exe')
$compiler = Join-Path $OutputDirectory 'advance-aot-supplement.exe'
$selected = @($manifest.libraries | Where-Object { $NativeProfile -eq 'all' -or $_.name -eq $NativeProfile })
$variantCounts = @{}
foreach ($entry in $selected) {
    $output = Join-Path $OutputDirectory ($entry.name + '.dll')
    & $compiler (Join-Path $ProjectRoot $entry.coverage) $Rom $output $Gcc --ram-variants | Tee-Object -Variable nativeOutput
    if ($LASTEXITCODE -ne 0) { throw "Native build failed: $($entry.name)" }
    $counts = [regex]::Match(($nativeOutput -join "`n"), 'RAM AOT supplement: (\d+) guarded instruction variants; (\d+) unsupported skipped')
    if (!$counts.Success -or $counts.Groups[2].Value -ne '0') { throw "Incomplete native profile: $($entry.name)" }
    $variantCounts[$entry.name] = [uint64]$counts.Groups[1].Value
}
if ($Install) {
    foreach ($entry in $selected) {
        Copy-Item -LiteralPath (Join-Path $OutputDirectory ($entry.name + '.dll')) -Destination (Join-Path $ProjectRoot $entry.path) -Force
        Copy-Item -LiteralPath (Join-Path $OutputDirectory ($entry.name + '.c')) -Destination (Join-Path $ProjectRoot $entry.source) -Force
        $hash = (Get-FileHash -LiteralPath (Join-Path $ProjectRoot $entry.path)).Hash.ToLowerInvariant()
        $entry | Add-Member -NotePropertyName sha256 -NotePropertyValue $hash -Force
        $entry | Add-Member -NotePropertyName variants -NotePropertyValue $variantCounts[$entry.name] -Force
    }
    $manifest | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
}
Write-Host "Native output: $OutputDirectory"
