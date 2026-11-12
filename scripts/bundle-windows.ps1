param([string]$Target = "x86_64-pc-windows-msvc")
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$StageRoot = Join-Path $ProjectRoot ("dist/.bundle-" + [guid]::NewGuid().ToString("N"))

# Code signing is optional: set WINDOWS_CERTIFICATE (base64 PFX) and
# WINDOWS_CERTIFICATE_PASSWORD to sign Pecan.exe and the installer.
function Find-SignTool {
    $Command = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($Command) { return $Command.Source }
    $Kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits/10/bin"
    $Found = Get-ChildItem $Kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match "\\x64\\" } |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $Found) { throw "signtool.exe was not found; install the Windows SDK" }
    return $Found.FullName
}
function Invoke-Sign([string]$Path) {
    if (-not $env:WINDOWS_CERTIFICATE) { return }
    $Tool = Find-SignTool
    & $Tool sign /f $script:Certificate /p $env:WINDOWS_CERTIFICATE_PASSWORD `
        /fd SHA256 /td SHA256 /tr "http://timestamp.digicert.com" /d "Pecan" $Path
    if ($LASTEXITCODE -ne 0) { throw "Signing $Path failed" }
    & $Tool verify /pa $Path
    if ($LASTEXITCODE -ne 0) { throw "Signature verification of $Path failed" }
}
function Find-Iscc {
    $Command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($Command) { return $Command.Source }
    foreach ($Root in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA "Programs"))) {
        $Candidate = Join-Path $Root "Inno Setup 6/ISCC.exe"
        if (Test-Path $Candidate) { return $Candidate }
    }
    return $null
}

Push-Location $ProjectRoot
try {
    $Version = (& python scripts/release-version.py).Trim()
    if ($LASTEXITCODE -ne 0) { throw "Cannot read release version" }
    # Windows file versions are four numbers; pre-release labels are dropped.
    $Numeric = ($Version -split "[-+]")[0] + ".0"
    & cargo build --locked --release --bin Pecan --target $Target
    if ($LASTEXITCODE -ne 0) { throw "Windows build failed" }
    $Name = "Pecan-$Version-Windows-x64"
    $Staging = Join-Path $StageRoot $Name
    New-Item -ItemType Directory -Force $Staging | Out-Null
    if ($env:WINDOWS_CERTIFICATE) {
        $script:Certificate = Join-Path $StageRoot "certificate.pfx"
        [IO.File]::WriteAllBytes($script:Certificate, [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE))
    }
    Copy-Item "target/$Target/release/Pecan.exe" "$Staging/Pecan.exe" -Force
    Copy-Item LICENSE "$Staging/LICENSE.txt" -Force
    Copy-Item NOTICE "$Staging/NOTICE.txt" -Force
    Copy-Item licenses "$Staging/licenses" -Recurse -Force
    $Executable = Get-Item "$Staging/Pecan.exe"
    if ($Executable.VersionInfo.ProductVersion -ne $Version) {
        throw "Executable resource version does not match Cargo.toml"
    }
    Invoke-Sign "$Staging/Pecan.exe"

    $Archive = Join-Path $ProjectRoot "dist/$Name.zip"
    $StagedArchive = Join-Path $StageRoot "$Name.zip"
    Compress-Archive -Path $Staging -DestinationPath $StagedArchive
    Move-Item -Path $StagedArchive -Destination $Archive -Force
    Write-Output $Archive

    $Iscc = Find-Iscc
    if ($Iscc) {
        Copy-Item assets/brand/pecan.ico "$Staging/pecan.ico" -Force
        $Installers = Join-Path $StageRoot "installer"
        & $Iscc /Q "/DVersion=$Version" "/DNumericVersion=$Numeric" "/DSource=$Staging" "/DOutput=$Installers" scripts/pecan.iss
        # The zip is the primary package; an installer failure is reported without blocking it.
        if ($LASTEXITCODE -ne 0) {
            Write-Output "::warning::Inno Setup failed ($LASTEXITCODE); only the zip archive was built"
        } else {
            $Installer = Join-Path $Installers "$Name-Setup.exe"
            Invoke-Sign $Installer
            $Destination = Join-Path $ProjectRoot "dist/$Name-Setup.exe"
            Move-Item -Path $Installer -Destination $Destination -Force
            Write-Output $Destination
        }
    } else {
        Write-Warning "Inno Setup 6 not found; only the zip archive was built"
    }
} finally {
    if (Test-Path $StageRoot) { Remove-Item $StageRoot -Recurse -Force }
    Pop-Location
}
