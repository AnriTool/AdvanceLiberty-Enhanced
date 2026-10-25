$ErrorActionPreference = 'Stop'
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
. (Join-Path $root 'tools/Native.ps1')
$fixture = Join-Path $root ('artifacts/native-install-test-' + [guid]::NewGuid().ToString('N'))
foreach ($name in @('native/usa','build-one','build-two')) { [void](New-Item -ItemType Directory -Path (Join-Path $fixture $name)) }
function Put([string]$path,[string]$text) { [IO.File]::WriteAllText((Join-Path $fixture $path),$text) }
function Assert($condition,[string]$message) { if (!$condition) { throw $message } }
Put 'native/usa/sessions.dll' 'old dll'
Put 'native/usa/sessions.c' 'old c'
$manifestPath = Join-Path $fixture 'native/usa/manifest.json'
$entry = [pscustomobject]@{name='sessions'; path='native/usa/sessions.dll'; source='native/usa/sessions.c'; variants=1; sha256='old'}
$manifest = [pscustomobject]@{format=1; libraries=@($entry)}
Put 'native/usa/manifest.json' ($manifest | ConvertTo-Json -Depth 5)
Put 'build-one/sessions.dll' 'new dll'
Put 'build-one/sessions.c' 'new c'
Install-NativeProfiles -Manifest $manifest -Selected @($entry) -VariantCounts @{sessions=2} -OutputDirectory (Join-Path $fixture 'build-one') -ProjectRoot $fixture
Assert (([IO.File]::ReadAllText((Join-Path $fixture 'native/usa/sessions.dll'))) -eq 'new dll') 'DLL not installed.'
Assert (([IO.File]::ReadAllText((Join-Path $fixture 'build-one/before-install/sessions.dll'))) -eq 'old dll') 'Previous DLL not backed up.'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
Assert ($manifest.libraries[0].variants -eq 2) 'Manifest not updated.'
Assert ($manifest.libraries[0].sha256 -eq (Get-FileHash (Join-Path $fixture 'native/usa/sessions.dll')).Hash.ToLowerInvariant()) 'Manifest hash differs.'
$before = @{}
foreach ($name in @('sessions.dll','sessions.c','manifest.json')) { $before[$name]=(Get-FileHash (Join-Path $fixture "native/usa/$name")).Hash }
Put 'build-two/sessions.dll' 'third dll'
Put 'build-two/sessions.c' 'third c'
# Reading remains possible for the backup, but replacing the second file fails.
$lock = [IO.File]::Open((Join-Path $fixture 'native/usa/sessions.c'),[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
$failed = $false
try {
    try { Install-NativeProfiles -Manifest $manifest -Selected $manifest.libraries -VariantCounts @{sessions=3} -OutputDirectory (Join-Path $fixture 'build-two') -ProjectRoot $fixture }
    catch { $failed=$true; Assert ($_.Exception.Message -match 'rolled back') 'Failure did not report successful rollback.' }
} finally { $lock.Dispose() }
Assert $failed 'Locked file should prevent installation.'
foreach ($name in $before.Keys) { Assert ((Get-FileHash (Join-Path $fixture "native/usa/$name")).Hash -eq $before[$name]) "Rollback did not restore $name" }
Assert (@(Get-ChildItem (Join-Path $fixture 'native/usa') -Filter '*.tmp').Count -eq 0) 'Staged files leaked.'
Write-Host "PASS: native installation, hashes, backups, rollback after partial installation, temporary cleanup. $fixture"
