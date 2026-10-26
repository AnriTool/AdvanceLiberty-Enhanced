param(
    [switch]$SkipBuild,
    [switch]$PrepareOnly,
    [uint64]$Frames = 0
)
. (Join-Path $PSScriptRoot '../Project.ps1')

# Keep menu experiments, including the game's EEPROM writes, off the live profile.
$probeDirectory = Join-Path $ProjectRoot ('artifacts/menu-play-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $probeDirectory | Out-Null
$profileFiles = @('data/saves/usa.sav', 'data/config/controls.cfg', 'data/config/enhanced.cfg')
$before = @($profileFiles | ForEach-Object {
    $source = Join-Path $ProjectRoot $_
    if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing profile file: $source" }
    Get-FileHash -LiteralPath $source
    Copy-Item -LiteralPath $source -Destination (Join-Path $probeDirectory (Split-Path $_ -Leaf))
})
$before | Select-Object Path, Hash | ConvertTo-Json | Set-Content (Join-Path $probeDirectory 'user-before.json')
Write-Host "Menu research directory: $probeDirectory"
if ($PrepareOnly) { return }

$probeExe = Get-GameExecutable -SkipBuild:$SkipBuild
$probeRom = Get-GameRom
$probeLibraries = Get-NativeLibraries
Write-Host 'Explore the original menus. Close with Esc > Quit game or the window close button.'
Write-Host 'All save/config changes go to the research copy. Session and text trace are recorded.'
try {
    $probeArguments = @($probeRom, '-', "$Frames", '-', 'play') + @($probeLibraries) + @(
        '--save', (Join-Path $probeDirectory 'usa.sav'),
        '--controls', (Join-Path $probeDirectory 'controls.cfg'),
        '--record-session', (Join-Path $probeDirectory 'session'),
        '--text-trace', (Join-Path $probeDirectory 'texts.csv'),
        '--control-state', (Join-Path $probeDirectory 'controls.csv'),
        '--camera-preview', '--original-menus'
    )
    # Start-Process joins ArgumentList on Windows. Quote each file path so spaces
    # survive, and redirect at the process level: Windows PowerShell otherwise
    # turns normal Rust stderr diagnostics into terminating NativeCommandError.
    $probeCommandLine = ($probeArguments | ForEach-Object { '"' + $_ + '"' }) -join ' '
    $probeProcess = Start-Process -FilePath $probeExe -ArgumentList $probeCommandLine `
        -NoNewWindow -Wait -PassThru `
        -RedirectStandardError (Join-Path $probeDirectory 'runtime.log') `
        -RedirectStandardOutput (Join-Path $probeDirectory 'stdout.log')
    if ($probeProcess.ExitCode -ne 0) { throw "Game exited with error $($probeProcess.ExitCode). See $probeDirectory/runtime.log" }
} finally {
    $changed = @($before | Where-Object { (Get-FileHash -LiteralPath $_.Path).Hash -ne $_.Hash })
    if ($changed.Count) { Write-Warning 'Live profile files changed during the run; inspect user-before.json.' }
    else { Write-Host 'Verified: live save and configs are unchanged.' }
    Write-Host "Results: $probeDirectory"
}
