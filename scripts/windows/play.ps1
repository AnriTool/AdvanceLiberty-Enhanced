param(
    [string]$Rom,
    [string]$SavePath,
    [string]$ControlsPath,
    [string]$LanguagePack,
    [switch]$SkipBuild,
    [switch]$NoRecord,
    [string]$SessionDirectory,
    [switch]$OriginalRenderer,
    [switch]$OriginalMenus,
    [switch]$EnhancedAudio = $true,
    [switch]$OriginalAudio,
    [switch]$PreserveCash,
    [ValidateRange(0,300)][uint32]$DropLifetime = 30,
    [switch]$AffineTextures,
    [uint64]$Frames = 0
)
. (Join-Path $PSScriptRoot 'Project.ps1')
if ($NoRecord -and $SessionDirectory) { throw 'Choose -NoRecord or -SessionDirectory.' }
$Rom = Get-GameRom $Rom
$libraries = Get-NativeLibraries
$exe = Get-GameExecutable -SkipBuild:$SkipBuild
if (!$SavePath) { $SavePath = Join-Path $ProjectRoot 'data/saves/usa.sav' }
if (!$ControlsPath) { $ControlsPath = Join-Path $ProjectRoot 'data/config/controls.cfg' }
$SavePath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SavePath)
$ControlsPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($ControlsPath)
foreach ($path in @($SavePath, $ControlsPath)) {
    New-Item -ItemType Directory -Force -Path (Split-Path $path -Parent) | Out-Null
}
$runtimeOptions = @('--camera-preview')
if ($EnhancedAudio -and !$OriginalAudio) { $runtimeOptions += '--enhanced-audio' }
if ($PreserveCash) { $runtimeOptions += '--preserve-cash' }
if ($DropLifetime -gt 0) { $runtimeOptions += @('--drop-lifetime', "$DropLifetime") }
if ($OriginalMenus) { $runtimeOptions += '--original-menus' }
if ($LanguagePack) { $runtimeOptions += @('--language-pack', (Resolve-Path -LiteralPath $LanguagePack).Path) }
if ($OriginalRenderer) { $runtimeOptions += '--original-renderer' }
if ($AffineTextures) { $runtimeOptions += '--affine-textures' }
if (!$NoRecord) {
    if (!$SessionDirectory) { $SessionDirectory = Join-Path $ProjectRoot ('data/sessions/session-' + [Guid]::NewGuid().ToString('N')) }
    $SessionDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SessionDirectory)
    New-Item -ItemType Directory -Force -Path (Split-Path $SessionDirectory -Parent) | Out-Null
    $runtimeOptions += @('--record-session', $SessionDirectory)
    Write-Host "Session: $SessionDirectory"
}
Write-Host "Save: $SavePath"
Write-Host "Controls: $ControlsPath"
Write-Host 'Esc / Pause: game menu. Enhanced Options: PC settings. F6: renderer; F8: controls; F11: fullscreen.'
$exitCode = Invoke-GameRuntime -Executable $exe -Arguments (@($Rom, '-', "$Frames", '-', 'play') + @($libraries) + @('--save', $SavePath, '--controls', $ControlsPath) + $runtimeOptions)
if ($exitCode -ne 0) { throw "Game exited with error $exitCode." }
