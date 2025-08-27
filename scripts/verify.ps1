param([switch]$Offline, [switch]$Package)
$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location -LiteralPath $RepoRoot
try {
    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed' }
    $CargoOptions = @('--locked', '--all-targets', '--jobs', '2', '--target-dir', (Join-Path $RepoRoot 'target'))
    if ($Offline) { $CargoOptions += '--offline' }
    & cargo clippy @CargoOptions -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
    & cargo test @CargoOptions
    if ($LASTEXITCODE -ne 0) { throw 'Fixture tests failed' }
    if ($Package) { & (Join-Path $PSScriptRoot 'package.ps1') -Offline:$Offline }
}
finally { Pop-Location }
