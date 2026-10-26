$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent

function Get-GameRom([string]$Rom) {
    if (!$Rom) { $Rom = Join-Path $ProjectRoot 'roms/usa.gba' }
    if (!(Test-Path -LiteralPath $Rom -PathType Leaf)) {
        throw "USA ROM not found: $Rom. Place your ROM in roms/usa.gba or use -Rom PATH."
    }
    (Resolve-Path -LiteralPath $Rom).Path
}

function Get-NativeLibraries {
    $manifest = Get-Content -LiteralPath (Join-Path $ProjectRoot 'native/usa/manifest.json') -Raw | ConvertFrom-Json
    @($manifest.libraries | ForEach-Object {
        $path = Join-Path $ProjectRoot $_.path
        if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw "Native library missing: $path" }
        (Resolve-Path -LiteralPath $path).Path
    })
}

function Get-GameExecutable([switch]$SkipBuild) {
    $exe = Join-Path $ProjectRoot 'target/release/AdvanceLiberty.exe'
    if (!$SkipBuild -or !(Test-Path -LiteralPath $exe)) {
        & (Join-Path $ProjectRoot 'scripts/windows/build.ps1') | Out-Host
    }
    $exe
}

# Windows PowerShell does not synchronously wait for GUI-subsystem programs
# invoked with '&'. Keep developer tools synchronous and capture both streams
# without treating ordinary Rust stderr as a terminating PowerShell error.
function ConvertTo-WindowsArgument([AllowEmptyString()][string]$Value) {
    $escaped = [regex]::Replace($Value, '(\\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
    '"' + $escaped + '"'
}
function Invoke-GameRuntime([string]$Executable, [string[]]$Arguments, [string]$LogPath) {
    $info = New-Object System.Diagnostics.ProcessStartInfo
    $info.FileName = $Executable
    $info.Arguments = ($Arguments | ForEach-Object { ConvertTo-WindowsArgument $_ }) -join ' '
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $info
    try {
        if (!$process.Start()) { throw 'Cannot start runtime.' }
        $output = $process.StandardOutput.ReadToEndAsync()
        $errors = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        if ($LogPath) {
            [IO.File]::WriteAllText($LogPath, $output.Result + $errors.Result)
        } else {
            if ($output.Result) { Write-Host $output.Result.TrimEnd() }
            if ($errors.Result) { Write-Host $errors.Result.TrimEnd() }
        }
        return $process.ExitCode
    } finally { $process.Dispose() }
}
