param(
    [Parameter(Mandatory = $true)][string]$Session,
    [string]$OutputDirectory,
    [string]$Rom,
    [switch]$SkipBuild,
    [switch]$AllowInterpreter
)
. (Join-Path $PSScriptRoot 'tools/Project.ps1')
if (!(Test-Path -LiteralPath $Session -PathType Container)) { $Session = Join-Path $ProjectRoot "data/sessions/$Session" }
$Session = (Resolve-Path -LiteralPath $Session).Path
$summary = @{}
Get-Content -LiteralPath (Join-Path $Session 'summary.txt') | ForEach-Object {
    $pair = $_ -split '=', 2
    if ($pair.Length -eq 2) { $summary[$pair[0]] = $pair[1] }
}
if (!$summary.ContainsKey('frames') -or [uint64]$summary.frames -eq 0) { throw 'Completed session summary required.' }
$Rom = Get-GameRom $Rom
$libraries = Get-NativeLibraries
$exe = Get-GameExecutable -SkipBuild:$SkipBuild
if (!$OutputDirectory) { $OutputDirectory = Join-Path $ProjectRoot ('artifacts/replays/' + (Split-Path $Session -Leaf) + '-' + [Guid]::NewGuid().ToString('N')) }
$OutputDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Replay output must be a new directory.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$save = Join-Path $OutputDirectory 'replay.sav'
Copy-Item -LiteralPath (Join-Path $Session 'initial.sav') -Destination $save
$actual = Join-Path $OutputDirectory 'frames.csv'
$options = @('--save', $save, '--audio-hashes', '--audit', (Join-Path $OutputDirectory 'fallbacks.csv'), '--camera-preview', '--preview-output', (Join-Path $OutputDirectory 'camera'))
if (!$AllowInterpreter) { $options += '--require-native' }
$exitCode = Invoke-GameRuntime -Executable $exe -Arguments (@($Rom, (Join-Path $Session 'input.gba-input'), $summary.frames, $actual, 'multi') + @($libraries) + $options)
if ($exitCode -ne 0) { throw "Replay failed ($exitCode); see $OutputDirectory" }
if ((Get-FileHash -LiteralPath $actual).Hash -ne (Get-FileHash -LiteralPath (Join-Path $Session 'frames.csv')).Hash) { throw 'Recorded video/audio hashes differ.' }
Write-Host "MATCH: $($summary.frames) recorded video/audio frames. Output: $OutputDirectory"
