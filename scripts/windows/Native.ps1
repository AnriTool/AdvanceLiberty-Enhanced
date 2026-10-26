# File installation is separate from compilation so rollback can be tested without GCC.
function Install-NativeProfiles {
    param($Manifest, $Selected, $VariantCounts, [string]$OutputDirectory, [string]$ProjectRoot)
    $ErrorActionPreference = 'Stop'
    $manifestPath = Join-Path $ProjectRoot 'native/usa/manifest.json'
    $backup = Join-Path $OutputDirectory 'before-install'
    [void](New-Item -ItemType Directory -Path $backup)
    Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $backup 'manifest.json')
    $files = @()
    foreach ($entry in $selected) {
        foreach ($extension in @('dll','c')) {
            $name = $entry.name + '.' + $extension
            $relative = $entry.path
            if ($extension -eq 'c') { $relative = $entry.source }
            $destination = Join-Path $ProjectRoot $relative
            $existed = Test-Path -LiteralPath $destination
            if ($existed) { Copy-Item -LiteralPath $destination -Destination (Join-Path $backup $name) }
            $files += [pscustomobject]@{Name=$name; Destination=$destination; Existed=$existed}
        }
    }
    $changed = @()
    try {
        foreach ($file in $files) {
            $staged = $file.Destination + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
            try {
                Copy-Item -LiteralPath (Join-Path $OutputDirectory $file.Name) -Destination $staged
                if ($file.Existed) { [IO.File]::Replace($staged, $file.Destination, [NullString]::Value) }
                else { [IO.File]::Move($staged, $file.Destination) }
                $changed += $file
            } finally {
                if (Test-Path -LiteralPath $staged) { Remove-Item -LiteralPath $staged }
            }
            if ((Get-FileHash -LiteralPath $file.Destination).Hash -ne (Get-FileHash -LiteralPath (Join-Path $OutputDirectory $file.Name)).Hash) { throw 'Installed file hash differs.' }
        }
        foreach ($entry in $selected) {
            $entry | Add-Member -NotePropertyName sha256 -NotePropertyValue (Get-FileHash -LiteralPath (Join-Path $ProjectRoot $entry.path)).Hash.ToLowerInvariant() -Force
            $entry | Add-Member -NotePropertyName variants -NotePropertyValue $variantCounts[$entry.name] -Force
        }
        [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 6) + "`n", (New-Object Text.UTF8Encoding($false)))
    } catch {
        $failure = $_
        $rollbackErrors = @()
        foreach ($file in $changed) {
            try {
                if ($file.Existed) { Copy-Item -LiteralPath (Join-Path $backup $file.Name) -Destination $file.Destination -Force }
                else { Remove-Item -LiteralPath $file.Destination }
            } catch { $rollbackErrors += $_.Exception.Message }
        }
        try { Copy-Item -LiteralPath (Join-Path $backup 'manifest.json') -Destination $manifestPath -Force }
        catch { $rollbackErrors += $_.Exception.Message }
        if ($rollbackErrors.Count) { throw "Native install failed; rollback incomplete. Restore files from $backup. $failure; $($rollbackErrors -join '; ')" }
        throw "Native install failed and was rolled back. Close the game before updating. Backup: $backup. $failure"
    }
    Write-Host "Installed native profiles. Previous files: $backup"
}
