$ErrorActionPreference = 'Stop'
$root = Split-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) -Parent
$fixture = Join-Path $root ('artifacts/native-coverage-test-' + [guid]::NewGuid().ToString('N'))
foreach ($directory in @('native/usa/coverage','roms','data/sessions/good','data/sessions/bad','data/sessions/unfinished')) {
    [void](New-Item -ItemType Directory -Path (Join-Path $fixture $directory))
}
function Put([string]$name,[string]$text) { [IO.File]::WriteAllText((Join-Path $fixture $name),$text,(New-Object Text.UTF8Encoding($false))) }
function Assert($condition,[string]$message) { if (!$condition) { throw $message } }
function Row([string]$key,[string]$opcode) { "dispatch,$key,1,0,1," + $opcode.PadRight(64,'0') }
$rom = New-Object byte[] 16777216
$rom[0]=1; $rom[1]=2; $rom[2]=3; $rom[3]=4
[IO.File]::WriteAllBytes((Join-Path $fixture 'roms/usa.gba'),$rom)
$sha = (Get-FileHash (Join-Path $fixture 'roms/usa.gba')).Hash.ToLowerInvariant()
$header = "reason,key,steps,first_frame,last_frame,bytes32`n"
Put 'native/usa/coverage/precise.csv' ($header + (Row '08000001' '0102') + "`n")
Put 'native/usa/coverage/sessions.csv' ($header + (Row '03000001' 'aabb') + "`n")
$manifest = @{rom_sha256=$sha; libraries=@(@{name='precise'; coverage='native/usa/coverage/precise.csv'},@{name='sessions'; coverage='native/usa/coverage/sessions.csv'; variants=1})}
Put 'native/usa/manifest.json' ($manifest | ConvertTo-Json -Depth 5)
foreach ($name in @('good','bad')) { Put "data/sessions/$name/summary.txt" "frames=2`ninterpreted_steps=5`n" }
Put 'data/sessions/good/fallbacks.csv' ($header + ((@((Row '03000001' 'aabb'),(Row '03000001' 'ccdd'),(Row '03000001' 'ccdd'),(Row '03000001' 'eeff'),(Row '08000003' '0304'))) -join "`n") + "`n")
Put 'data/sessions/bad/fallbacks.csv' ($header + (Row '03000003' '1234') + "`n" + (Row '08000003' 'dead') + "`n")
Put 'data/sessions/unfinished/fallbacks.partial.csv' ($header + (Row '03000003' '5678') + "`n")
$target = Join-Path $fixture 'native/usa/coverage/sessions.csv'
$before = (Get-FileHash $target).Hash
$collector = Join-Path $root 'scripts/windows/collect-native.ps1'
$preview = & $collector -ProjectDirectory $fixture -DryRun
Assert ($preview.Added -eq 3) 'Deduplication lost distinct RAM variants or included duplicate/known rows.'
Assert ($preview.ExcludedSessions.Count -eq 2) 'Invalid and unfinished sessions must be excluded.'
Assert ((Get-FileHash $target).Hash -eq $before) 'Dry run changed coverage.'
$report = & $collector -ProjectDirectory $fixture
Assert ($report.Added -eq 3 -and $report.Variants -eq 4) 'Wrong merge result.'
$rows = @(Import-Csv $target)
Assert ($rows.Count -eq 4) 'Wrong output row count.'
Assert (@($rows | Where-Object key -eq '03000003').Count -eq 0) 'Part of a rejected session leaked into coverage.'
$again = & $collector -ProjectDirectory $fixture
Assert ($again.Added -eq 0) 'Repeated collection must be idempotent.'
$mergedHash = (Get-FileHash $target).Hash
$rejected = $false
try { & $collector -ProjectDirectory $fixture -Session (Join-Path $fixture 'data/sessions/bad') | Out-Null }
catch { $rejected = $true }
Assert $rejected 'Explicit incompatible session must fail.'
Assert ((Get-FileHash $target).Hash -eq $mergedHash) 'Rejected collection changed coverage.'
Put 'roms/wrong.gba' 'wrong ROM'
$rejected = $false
try { & $collector -ProjectDirectory $fixture -Rom (Join-Path $fixture 'roms/wrong.gba') | Out-Null }
catch { $rejected = $true }
Assert $rejected 'Wrong ROM hash must fail.'
Write-Host "PASS: native coverage deduplication, RAM variants, ROM validation, whole-session rejection, incomplete sessions, dry run, idempotence. $fixture"
