param([string[]]$Session, [string]$Rom, [string]$Gcc, [switch]$Verify, [switch]$SkipBuild, [switch]$Force)
. (Join-Path $PSScriptRoot 'Project.ps1')
$Rom = Get-GameRom $Rom
$report = & (Join-Path $PSScriptRoot 'collect-native.ps1') -Session $Session -Rom $Rom
$manifest = Get-Content -LiteralPath (Join-Path $ProjectRoot 'native/usa/manifest.json') -Raw | ConvertFrom-Json
$installed = @($manifest.libraries | Where-Object name -eq 'sessions')[0]
if (!$Force -and $report.Added -eq 0 -and $report.Variants -eq $installed.variants) {
    Write-Host 'No new coverage. Native DLLs are already up to date.'
    return
}
$options = @{NativeProfile='sessions'; Rom=$Rom; Install=$true; SkipBuild=$SkipBuild}
if ($Gcc) { $options.Gcc = $Gcc }
if ($Verify) {
    $paths = @($report.AcceptedSessions | Where-Object {$_.Added -gt 0} | ForEach-Object {$_.Session})
    if (!$paths.Count) { $paths = @($report.AcceptedSessions | Select-Object -First 1 | ForEach-Object {$_.Session}) }
    if (!$paths.Count) { throw 'No completed compatible sessions available for verification.' }
    $options.VerifySession = $paths
} else {
    Write-Host 'Fast update: recorded sessions will not be replayed. Use -Verify for video/audio verification.'
}
& (Join-Path $PSScriptRoot 'rebuild-native.ps1') @options
