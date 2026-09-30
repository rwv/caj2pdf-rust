# SPDX-License-Identifier: MIT
# Independent validators only; never included in converter release archives.
$ErrorActionPreference = 'Stop'
$tools = Join-Path $env:RUNNER_TEMP 'caj2pdf-validators'
New-Item -ItemType Directory -Force $tools | Out-Null
$variant = 'msvc64'
$qpdfHash = 'db87077e683630c1217e0e8f9a20a9749d952ab676e881c3689187763a5de25d'
if ($env:RUNNER_ARCH -eq 'ARM64') {
    $variant = 'msvc-arm64'
    $qpdfHash = '2b5d7740a2513d8a841116692dc2cf7efe83f35262386ebab18bed35fbf15ab1'
}
$downloads = @(
    @("https://github.com/qpdf/qpdf/releases/download/v12.4.2/qpdf-12.4.2-$variant.zip", $qpdfHash, 'qpdf'),
    @('https://github.com/ArtifexSoftware/mupdf-downloads/releases/download/1.28.5/mupdf-1.28.5-windows.zip', '13476a53e84f5527983158acb6442c7853b3cdc022f3b24880780dcfba1fcc99', 'mutool')
)
foreach ($item in $downloads) {
    $zip = Join-Path $tools "$($item[2]).zip"
    Invoke-WebRequest $item[0] -OutFile $zip
    if ((Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $item[1]) { throw 'Validator checksum mismatch' }
    $dest = Join-Path $tools $item[2]
    Expand-Archive $zip -DestinationPath $dest -Force
    $exe = Get-ChildItem $dest -Recurse -Filter "$($item[2]).exe" | Select-Object -First 1
    if (-not $exe) { throw 'Validator executable missing' }
    $exe.DirectoryName | Out-File -FilePath $env:GITHUB_PATH -Append -Encoding utf8
}
