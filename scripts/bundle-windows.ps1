param([string]$Target = "x86_64-pc-windows-msvc")
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$StageRoot = Join-Path $ProjectRoot ("dist/.bundle-" + [guid]::NewGuid().ToString("N"))
Push-Location $ProjectRoot
try {
    $Version = (& python scripts/release-version.py).Trim()
    if ($LASTEXITCODE -ne 0) { throw "Cannot read release version" }
    & cargo build --locked --release --bin Pecan --target $Target
    if ($LASTEXITCODE -ne 0) { throw "Windows build failed" }
    $Name = "Pecan-$Version-Windows-x64"
    $Staging = Join-Path $StageRoot $Name
    New-Item -ItemType Directory -Force $Staging | Out-Null
    Copy-Item "target/$Target/release/Pecan.exe" "$Staging/Pecan.exe" -Force
    Copy-Item LICENSE "$Staging/LICENSE.txt" -Force
    Copy-Item licenses "$Staging/licenses" -Recurse -Force
    $Executable = Get-Item "$Staging/Pecan.exe"
    if ($Executable.VersionInfo.ProductVersion -ne $Version) {
        throw "Executable resource version does not match Cargo.toml"
    }
    $Archive = Join-Path $ProjectRoot "dist/$Name.zip"
    $StagedArchive = Join-Path $StageRoot "$Name.zip"
    Compress-Archive -Path $Staging -DestinationPath $StagedArchive
    Move-Item -Path $StagedArchive -Destination $Archive -Force
    Write-Output $Archive
} finally {
    if (Test-Path $StageRoot) { Remove-Item $StageRoot -Recurse -Force }
    Pop-Location
}
