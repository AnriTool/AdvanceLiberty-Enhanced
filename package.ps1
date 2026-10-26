param([string]$OutputDirectory, [switch]$SkipBuild, [switch]$Offline)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'tools/Project.ps1')
if (!$SkipBuild) { & (Join-Path $PSScriptRoot 'build.ps1') -Offline:$Offline }
$exe = Get-GameExecutable -SkipBuild
$cargo = Get-Content (Join-Path $ProjectRoot 'Cargo.toml') -Raw
$version = [regex]::Match($cargo, '(?m)^version = "([^"]+)"').Groups[1].Value
if (!$version) { throw 'Package version missing.' }
if (!$OutputDirectory) { $OutputDirectory = Join-Path $ProjectRoot "dist/AdvanceLiberty-Enhanced-$version-win64" }
$OutputDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Choose a new output directory; existing release/user files are never overwritten.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
foreach ($dir in @('native/usa','roms','languages','licenses')) { New-Item -ItemType Directory -Path (Join-Path $OutputDirectory $dir) | Out-Null }
Copy-Item -LiteralPath $exe -Destination (Join-Path $OutputDirectory 'AdvanceLiberty.exe')
$manifest = Get-Content (Join-Path $ProjectRoot 'native/usa/manifest.json') -Raw | ConvertFrom-Json
$entries = @()
foreach ($entry in $manifest.libraries) {
    $source = Join-Path $ProjectRoot $entry.path
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry.sha256) { throw "Native checksum mismatch: $source" }
    $relative = 'native/usa/' + (Split-Path $entry.path -Leaf)
    Copy-Item -LiteralPath $source -Destination (Join-Path $OutputDirectory $relative)
    $entries += [ordered]@{name=$entry.name; path=$relative; sha256=$entry.sha256; variants=$entry.variants}
}
[ordered]@{format=1; rom_sha256=$manifest.rom_sha256; libraries=$entries} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'native/usa/manifest.json') -Encoding UTF8
# PowerShell 5.1 writes a BOM; serde_json expects JSON without it.
$jsonPath = Join-Path $OutputDirectory 'native/usa/manifest.json'
[IO.File]::WriteAllText($jsonPath, ([IO.File]::ReadAllText($jsonPath)), (New-Object Text.UTF8Encoding($false)))
Copy-Item (Join-Path $ProjectRoot 'release/README.txt') (Join-Path $OutputDirectory 'README.txt')
Copy-Item (Join-Path $ProjectRoot 'release/README.ru.txt') (Join-Path $OutputDirectory 'README.ru.txt')
Copy-Item (Join-Path $ProjectRoot 'LICENSE') (Join-Path $OutputDirectory 'LICENSE')
Copy-Item (Join-Path $ProjectRoot 'NOTICE') (Join-Path $OutputDirectory 'NOTICE')
Copy-Item (Join-Path $ProjectRoot 'release/THIRD_PARTY_NOTICES.txt') (Join-Path $OutputDirectory 'licenses/THIRD_PARTY_NOTICES.txt')
Copy-Item (Join-Path $ProjectRoot 'vendor/LICENSE-MIT') (Join-Path $OutputDirectory 'licenses/gba-recomp-MIT.txt')
Copy-Item (Join-Path $ProjectRoot 'vendor/LICENSE-APACHE') (Join-Path $OutputDirectory 'licenses/gba-recomp-APACHE.txt')
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'roms/PUT_YOUR_ROM_HERE.txt'), "Put your own legally obtained, unmodified USA .gba dump here. Any filename is accepted. No ROM is included or downloaded.")
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'languages/README.txt'), "Language packs go here. Editable en.json is generated from your own ROM on first launch. Select packs in Enhanced Options / Language.")
Push-Location $ProjectRoot
try {
    $metadataText = & cargo metadata --offline --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Cannot collect dependency licenses.' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
} finally { Pop-Location }
$dependencies = @()
foreach ($dependency in $metadata.packages | Where-Object source) {
    $id = $dependency.name + '-' + $dependency.version
    $source = Split-Path $dependency.manifest_path -Parent
    $licenseDir = Join-Path $OutputDirectory "licenses/$id"
    New-Item -ItemType Directory -Path $licenseDir | Out-Null
    Get-ChildItem -LiteralPath $source -File | Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)' | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $licenseDir $_.Name)
    }
    if ($dependency.name -eq 'sdl2-sys') { Copy-Item (Join-Path $source 'SDL/LICENSE.txt') (Join-Path $licenseDir 'SDL-LICENSE.txt') }
    $dependencies += [ordered]@{name=$dependency.name; version=$dependency.version; license=$dependency.license}
}
$dependencies | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $OutputDirectory 'licenses/dependencies.json') -Encoding UTF8
# Explicit whitelist: no ROM, user languages, saves, sessions, C snapshots,
# coverage, development tools, scripts or development config are copied.
$files = @(Get-ChildItem -LiteralPath $OutputDirectory -File -Recurse)
if ($files | Where-Object Extension -In @('.gba','.sav','.ps1','.bat','.cmd','.c','.csv','.pdb')) { throw 'Unexpected development or user content in release.' }
# Some registry archives preserve Unix epoch timestamps, outside ZIP's range.
# Normalize only packaged copies; dependency/source timestamps stay untouched.
foreach ($file in $files) {
    if ($file.LastWriteTime.Year -lt 1980) { $file.LastWriteTime = [datetime]'2000-01-01' }
}
$hashes = @($files | ForEach-Object { [ordered]@{path=$_.FullName.Substring($OutputDirectory.Length+1).Replace('\','/'); sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()} })
[ordered]@{product='AdvanceLiberty: Enhanced'; version=$version; platform='windows-x64'; files=$hashes} | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $OutputDirectory 'release.json') -Encoding UTF8
$zip = "$OutputDirectory.zip"
if (Test-Path -LiteralPath $zip) { throw 'Archive already exists; choose a new output directory.' }
Compress-Archive -LiteralPath $OutputDirectory -DestinationPath $zip -CompressionLevel Optimal
(Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + (Split-Path $zip -Leaf) | Set-Content "$zip.sha256" -Encoding ASCII
Write-Host "Release folder: $OutputDirectory"
Write-Host "Archive: $zip"
