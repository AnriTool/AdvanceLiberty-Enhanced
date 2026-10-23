param([switch]$Offline, [switch]$Sdl)
$ErrorActionPreference = 'Stop'
$cargoOptions = @('test', '--release', '--locked', '--package', 'gta-advance', '--package', 'advance-aot-supplement')
if ($Offline) { $cargoOptions += '--offline' }
Push-Location $PSScriptRoot
$oldVideo = $env:SDL_VIDEODRIVER
$oldRenderer = $env:SDL_RENDER_DRIVER
try {
    & cargo @cargoOptions
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
    if ($Sdl) {
        $env:SDL_VIDEODRIVER = 'dummy'
        $env:SDL_RENDER_DRIVER = 'software'
        $sdlOptions = @('test', '--release', '--locked', '--package', 'gta-advance')
        if ($Offline) { $sdlOptions += '--offline' }
        & cargo @sdlOptions f_keys_switch_real_sdl_output_and_reuse_textures -- --ignored
        if ($LASTEXITCODE -ne 0) { throw 'SDL test failed.' }
    }
} finally {
    $env:SDL_VIDEODRIVER = $oldVideo
    $env:SDL_RENDER_DRIVER = $oldRenderer
    Pop-Location
}
