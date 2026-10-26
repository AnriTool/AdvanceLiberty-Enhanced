param(
    [Alias('Profile')][ValidateSet('precise', 'extra', 'mission', 'sessions', 'all')][string]$NativeProfile = 'all',
    [string]$Rom,
    [string]$Gcc,
    [string]$OutputDirectory,
    [string[]]$VerifySession,
    [switch]$SkipBuild,
    [switch]$Install
)
. (Join-Path $PSScriptRoot 'Project.ps1')
. (Join-Path $PSScriptRoot 'Native.ps1')
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
foreach ($sessionPath in $VerifySession) {
    if (!(Test-Path -LiteralPath $sessionPath -PathType Container)) { $sessionPath = Join-Path $ProjectRoot "data/sessions/$sessionPath" }
    $sessionPath = (Resolve-Path -LiteralPath $sessionPath).Path
    $summary = @{}
    Get-Content -LiteralPath (Join-Path $sessionPath 'summary.txt') | ForEach-Object {
        $pair = $_ -split '=',2
        if ($pair.Length -eq 2) { $summary[$pair[0]] = $pair[1] }
    }
    if (![uint64]$summary.frames) { throw 'Completed nonempty session required for verification.' }
    $replay = Join-Path $OutputDirectory ('verify-' + (Split-Path $sessionPath -Leaf) + '-' + [guid]::NewGuid().ToString('N'))
    [void](New-Item -ItemType Directory -Path $replay)
    Copy-Item -LiteralPath (Join-Path $sessionPath 'initial.sav') -Destination (Join-Path $replay 'replay.sav')
    $libraries = @($manifest.libraries | ForEach-Object {
        if ($variantCounts.ContainsKey($_.name)) { Join-Path $OutputDirectory ($_.name + '.dll') }
        else { Join-Path $ProjectRoot $_.path }
    })
    Write-Host "Verifying $sessionPath ($($summary.frames) frames)..."
    $exe = Join-Path $ProjectRoot 'target/release/AdvanceLiberty.exe'
    $exitCode = Invoke-GameRuntime -Executable $exe -LogPath (Join-Path $replay 'runtime.log') -Arguments (@($Rom, (Join-Path $sessionPath 'input.gba-input'), $summary.frames, (Join-Path $replay 'frames.csv'), 'multi') + @($libraries) + @('--save', (Join-Path $replay 'replay.sav'), '--audio-hashes', '--audit', (Join-Path $replay 'fallbacks.csv'), '--require-native'))
    if ($exitCode -ne 0) { throw "Native replay failed; active DLLs unchanged. See $replay" }
    if ((Get-FileHash -LiteralPath (Join-Path $replay 'frames.csv')).Hash -ne (Get-FileHash -LiteralPath (Join-Path $sessionPath 'frames.csv')).Hash) {
        throw "Video/audio hashes differ; active DLLs unchanged. See $replay"
    }
    Write-Host "MATCH: $($summary.frames) frames, no interpreter."
}
if ($Install) {
    Install-NativeProfiles -Manifest $manifest -Selected $selected -VariantCounts $variantCounts -OutputDirectory $OutputDirectory -ProjectRoot $ProjectRoot
}
Write-Host "Native output: $OutputDirectory"
