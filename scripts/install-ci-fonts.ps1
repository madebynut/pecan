# Reproducible CJK glyph coverage for headless rendering tests on English Windows runners.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Directory = Join-Path $env:LOCALAPPDATA 'Microsoft/Windows/Fonts'
New-Item -ItemType Directory -Force $Directory | Out-Null
$Commit = 'f8d157532fbfaeda587e826d4cd5b21a49186f7c'
$Fonts = @(
    @('Sans', 'NotoSansCJKsc-Regular.otf', '2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b'),
    @('Serif', 'NotoSerifCJKsc-Regular.otf', '2a2eae2628df83556c54018c41e20fa532c1b862c5256ae8b3f23feb918d12ca')
)
foreach ($Font in $Fonts) {
    $Destination = Join-Path $Directory $Font[1]
    $Url = "https://raw.githubusercontent.com/notofonts/noto-cjk/$Commit/$($Font[0])/OTF/SimplifiedChinese/$($Font[1])"
    Invoke-WebRequest -Uri $Url -OutFile $Destination
    if ((Get-FileHash $Destination -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Font[2]) {
        throw "Unexpected font checksum: $($Font[1])"
    }
}
