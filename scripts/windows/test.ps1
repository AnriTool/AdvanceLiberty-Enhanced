param([switch]$Offline, [switch]$Sdl)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Project.ps1')
$cargoOptions = @('test', '--release', '--locked', '--package', 'advance-liberty', '--package', 'advance-aot-supplement')
if ($Offline) { $cargoOptions += '--offline' }
Push-Location $ProjectRoot
$oldVideo = $env:SDL_VIDEODRIVER
$oldRenderer = $env:SDL_RENDER_DRIVER
$oldAudio = $env:SDL_AUDIODRIVER
try {
    & (Join-Path $PSScriptRoot 'tests/native-coverage.ps1')
    & (Join-Path $PSScriptRoot 'tests/native-install.ps1')
    & cargo @cargoOptions
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
    if ($Sdl) {
        $env:SDL_VIDEODRIVER = 'dummy'
        $env:SDL_RENDER_DRIVER = 'software'
        $env:SDL_AUDIODRIVER = 'dummy'
        $sdlOptions = @('test', '--release', '--locked', '--package', 'advance-liberty')
        if ($Offline) { $sdlOptions += '--offline' }
        foreach ($sdlTest in @('f_keys_switch_real_sdl_output_and_reuse_textures', 'options_menu_real_sdl', 'mouse_aim_options_and_viewport_real_sdl', 'language_options_real_sdl', 'native_menu_frontend_real_sdl', 'localized_pause_and_settings_real_sdl', 'enhanced_menu_audio_real_sdl')) {
            & cargo @sdlOptions $sdlTest -- --ignored
            if ($LASTEXITCODE -ne 0) { throw "SDL test failed: $sdlTest" }
        }
    }
} finally {
    $env:SDL_VIDEODRIVER = $oldVideo
    $env:SDL_RENDER_DRIVER = $oldRenderer
    $env:SDL_AUDIODRIVER = $oldAudio
    Pop-Location
}
