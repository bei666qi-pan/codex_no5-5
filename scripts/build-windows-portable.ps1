$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $root

$target = "x86_64-pc-windows-msvc"
$release = Join-Path $root "target\$target\release"
$staging = Join-Path $root "dist\codex-network-guard-windows-x64"
$archive = Join-Path $root "dist\codex-network-guard-windows-x64.zip"

cargo build --release --workspace --target $target

if (Test-Path $staging) {
  Remove-Item -Recurse -Force $staging
}
New-Item -ItemType Directory -Force $staging | Out-Null

foreach ($binary in @("cng-desktop.exe", "cng.exe", "cngd.exe", "cng-codex.exe")) {
  $source = Join-Path $release $binary
  if (!(Test-Path $source)) {
    throw "Expected binary was not built: $source"
  }
  Copy-Item $source (Join-Path $staging $binary)
}
Copy-Item README.md, LICENSE (Join-Path $staging ".")

if (Test-Path $archive) {
  Remove-Item -Force $archive
}
Compress-Archive -Path $staging -DestinationPath $archive -Force
(Get-FileHash -Algorithm SHA256 $archive).Hash.ToLower() + "  " + (Split-Path $archive -Leaf) |
  Set-Content -NoNewline "$archive.sha256"

Write-Host "Built $archive"
