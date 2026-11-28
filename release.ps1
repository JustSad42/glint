# Builds Glint and its installer, and optionally publishes them as a GitHub release.
#   .\release.ps1                          build + installer only (dist\)
#   .\release.ps1 -Publish -Notes "..."    also tag v<version> and create the GitHub release
# The version lives in rust\Cargo.toml ([workspace.package]).
param([switch]$Publish, [string]$Notes = "")
$ErrorActionPreference = "Stop"
$root = $PSScriptRoot

$version = (Select-String -Path "$root\rust\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "Glint $version"

Push-Location "$root\rust"
cargo test --release --workspace
if ($LASTEXITCODE) { Pop-Location; throw "tests failed" }
cargo build --release --workspace
if ($LASTEXITCODE) { Pop-Location; throw "cargo build failed" }
Pop-Location

$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found (winget install JRSoftware.InnoSetup)" }
& $iscc /Q "/DAppVersion=$version" "$root\installer\setup.iss"
if ($LASTEXITCODE) { throw "installer build failed" }

$setup = "$root\dist\Glint-$version-setup.exe"
Write-Host "Built $setup"

if ($Publish) {
    if (-not $Notes) { $Notes = "Glint $version" }
    gh release create "v$version" $setup --title "Glint $version" --notes $Notes
    if ($LASTEXITCODE) { throw "gh release create failed" }
}
