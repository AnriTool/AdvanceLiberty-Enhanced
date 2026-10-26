param(
    [string[]]$Session,
    [string]$Rom,
    [string]$ProjectDirectory = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    [string]$OutputDirectory,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath $ProjectDirectory).Path
$manifest = Get-Content -LiteralPath (Join-Path $root 'native/usa/manifest.json') -Raw | ConvertFrom-Json
if (!$Rom) { $Rom = Join-Path $root 'roms/usa.gba' }
if ((Get-FileHash -LiteralPath $Rom).Hash.ToLowerInvariant() -ne $manifest.rom_sha256) { throw 'Native coverage requires the verified USA ROM.' }
$romBytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Rom).Path)
$header = 'reason,key,steps,first_frame,last_frame,bytes32'
function Read-Audit([string]$path) {
    if ((Get-Content -LiteralPath $path -TotalCount 1) -ne $header) { throw "Invalid audit header: $path" }
    Import-Csv -LiteralPath $path
}
function Signature($row) {
    if ($row.key -notmatch '^[0-9a-fA-F]{8}$' -or $row.bytes32 -notmatch '^[0-9a-fA-F]{64}$') { throw 'Invalid instruction address/bytes.' }
    $key = [Convert]::ToUInt32($row.key, 16)
    $size = 4
    if ($key -band 1) { $size = 2 }
    $address = $key -band [uint32]4294967294
    if (($address % $size) -ne 0) { throw 'Misaligned instruction.' }
    $region = $key -shr 24
    if ($region -notin @(2, 3, 8)) { throw "Unsupported instruction region: $($row.key)" }
    [void][uint64]::Parse($row.steps)
    [void][uint64]::Parse($row.first_frame)
    [void][uint64]::Parse($row.last_frame)
    if ($region -eq 8) {
        $offset = $address -band 33554431
        if ($offset + $size -gt $romBytes.Length) { throw 'Instruction outside ROM.' }
        for ($i = 0; $i -lt $size; $i++) {
            if ($romBytes[$offset + $i] -ne [Convert]::ToByte($row.bytes32.Substring($i * 2, 2), 16)) {
                throw "ROM bytes differ at $($row.key)"
            }
        }
    }
    '{0:x8}:{1}' -f $key, $row.bytes32.Substring(0, $size * 2).ToLowerInvariant()
}
$known = New-Object 'Collections.Generic.HashSet[string]'
$merged = New-Object 'Collections.Generic.Dictionary[string,object]'
$target = @($manifest.libraries | Where-Object name -eq 'sessions')
if ($target.Count -ne 1) { throw 'Exactly one sessions profile required.' }
$coverage = Join-Path $root $target[0].coverage
foreach ($lib in $manifest.libraries) {
    foreach ($row in (Read-Audit (Join-Path $root $lib.coverage))) {
        # Historical base profiles can also contain rows ignored by the generator.
        if (([Convert]::ToUInt32($row.key,16) -shr 24) -notin @(2,3,8)) { continue }
        $id = Signature $row
        [void]$known.Add($id)
        if ($lib.name -eq 'sessions') { $merged[$id] = $row }
    }
}
$beforeCount = $merged.Count
$explicit = $Session -and $Session.Count -gt 0
if ($explicit) {
    $directories = @($Session | ForEach-Object {
        $path = $_
        if (!(Test-Path -LiteralPath $path -PathType Container)) { $path = Join-Path $root "data/sessions/$_" }
        Get-Item -LiteralPath $path
    } | Sort-Object FullName -Unique)
} else {
    $directories = @()
    if (Test-Path -LiteralPath (Join-Path $root 'data/sessions')) {
        $directories = @(Get-ChildItem -LiteralPath (Join-Path $root 'data/sessions') -Directory | Sort-Object LastWriteTime -Descending)
    }
}
$accepted = @()
$excluded = @()
foreach ($directory in $directories) {
    if (!(Test-Path -LiteralPath (Join-Path $directory.FullName 'summary.txt')) -or
        !(Test-Path -LiteralPath (Join-Path $directory.FullName 'fallbacks.csv'))) {
        if ($explicit) { throw "Incomplete session: $($directory.FullName)" }
        $excluded += [pscustomobject]@{Session=$directory.FullName; Reason='Incomplete session: final summary/audit missing.'}
        continue
    }
    try {
        $summary = @{}
        foreach ($line in (Get-Content -LiteralPath (Join-Path $directory.FullName 'summary.txt'))) {
            $pair = $line -split '=',2
            if ($pair.Length -eq 2) { $summary[$pair[0]] = $pair[1] }
        }
        $frames = [uint64]::Parse($summary.frames)
        $interpreted = [uint64]::Parse($summary.interpreted_steps)
        if (!$frames) { throw 'Empty session.' }
        # Only the final audit is accepted, never fallbacks.partial.csv.
        $rows = @(Read-Audit (Join-Path $directory.FullName 'fallbacks.csv'))
        $pending = New-Object 'Collections.Generic.Dictionary[string,object]'
        foreach ($row in $rows) {
            $id = Signature $row
            if (!$known.Contains($id)) { $pending[$id] = $row }
        }
        foreach ($id in $pending.Keys) {
            $merged[$id] = $pending[$id]
            [void]$known.Add($id)
        }
        $accepted += [pscustomobject]@{Session=$directory.FullName; Frames=$frames; Interpreted=$interpreted; Added=$pending.Count}
    } catch {
        if ($explicit) { throw "Rejected $($directory.FullName): $_" }
        $excluded += [pscustomobject]@{Session=$directory.FullName; Reason=$_.Exception.Message}
        Write-Warning "Excluded $($directory.Name): $($_.Exception.Message)"
    }
}
if (!$OutputDirectory) { $OutputDirectory = Join-Path $root ('artifacts/native-collect-' + [guid]::NewGuid().ToString('N')) }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Collection output must be a new directory.' }
[void](New-Item -ItemType Directory -Path $OutputDirectory)
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
Copy-Item -LiteralPath $coverage -Destination (Join-Path $OutputDirectory 'before-sessions.csv')
$added = $merged.Count - $beforeCount
if ($added -gt 0) {
    $lines = New-Object 'Collections.Generic.List[string]'
    $lines.Add($header)
    foreach ($id in ($merged.Keys | Sort-Object)) {
        $row = $merged[$id]
        $lines.Add(($row.reason,$row.key,$row.steps,$row.first_frame,$row.last_frame,$row.bytes32 -join ','))
    }
    $candidate = Join-Path $OutputDirectory 'sessions.csv'
    [IO.File]::WriteAllText($candidate, ($lines -join "`n") + "`n", (New-Object Text.UTF8Encoding($false)))
    if (!$DryRun) {
        $staged = "$coverage.$([guid]::NewGuid().ToString('N')).tmp"
        try {
            Copy-Item -LiteralPath $candidate -Destination $staged
            [IO.File]::Replace($staged, $coverage, [NullString]::Value)
        } finally {
            if (Test-Path -LiteralPath $staged) { Remove-Item -LiteralPath $staged }
        }
    }
}
$result = [pscustomobject]@{Added=$added; Variants=$merged.Count; Coverage=$coverage; DryRun=[bool]$DryRun;
    AcceptedSessions=$accepted; ExcludedSessions=$excluded; Directory=$OutputDirectory}
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'collection.json'), ($result | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding($false)))
Write-Host "Native coverage: +$added variants; sessions=$($merged.Count); excluded=$($excluded.Count)"
Write-Host "Collection report/backup: $OutputDirectory"
$result
