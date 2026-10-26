param([switch]$Offline, [switch]$Tools)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Project.ps1')
$cargoOptions = @('build', '--release', '--locked', '--package', 'advance-liberty', '--bin', 'AdvanceLiberty')
if ($Offline) { $cargoOptions += '--offline' }
Push-Location $ProjectRoot
try {
    & cargo @cargoOptions
    if ($LASTEXITCODE -ne 0) { throw "Build failed ($LASTEXITCODE)." }
    if ($Tools) {
        $toolOptions = @('build', '--release', '--locked', '--package', 'advance-aot-supplement')
        if ($Offline) { $toolOptions += '--offline' }
        & cargo @toolOptions
        if ($LASTEXITCODE -ne 0) { throw "Tool build failed ($LASTEXITCODE)." }
    }
} finally { Pop-Location }
