param([switch]$Offline)
$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location -LiteralPath $RepoRoot
try {
    $MetadataArgs = @('metadata', '--format-version', '1', '--locked')
    if ($Offline) { $MetadataArgs += '--offline' }
    $MetadataText = & cargo @MetadataArgs
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
    $Metadata = $MetadataText | ConvertFrom-Json
    $Project = $Metadata.packages | Where-Object { $_.name -eq 'usb-eject' }
    $ProductVersion = $Project.version
    $BuildArgs = @('build', '--release', '--locked', '--jobs', '2', '--target', 'x86_64-pc-windows-msvc', '--target-dir', (Join-Path $RepoRoot 'target'))
    if ($Offline) { $BuildArgs += '--offline' }
    & cargo @BuildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    $DistRoot = Join-Path $RepoRoot 'dist'
    New-Item -ItemType Directory -Path $DistRoot -Force | Out-Null
    $AssetName = "UsbEject-v$ProductVersion.exe"
    $AssetPath = Join-Path $DistRoot $AssetName
    Copy-Item -LiteralPath (Join-Path $RepoRoot 'target\x86_64-pc-windows-msvc\release\UsbEject.exe') -Destination $AssetPath
    $Hash = (Get-FileHash -LiteralPath $AssetPath -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $DistRoot 'SHA256SUMS'), "$Hash  $AssetName`n", [Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath (Join-Path $RepoRoot 'LICENSE') -Destination (Join-Path $DistRoot 'LICENSE')
    $Notices = [Text.StringBuilder]::new()
    [void]$Notices.AppendLine('UsbEject dependency license notices. Includes build-time dependencies.')
    foreach ($Package in ($Metadata.packages | Where-Object { $_.name -ne 'usb-eject' } | Sort-Object name, version)) {
        [void]$Notices.AppendLine("`n===== $($Package.name) $($Package.version) — $($Package.license) =====")
        $PackageRoot = Split-Path -Parent $Package.manifest_path
        $LicenseFiles = Get-ChildItem -LiteralPath $PackageRoot -File | Where-Object { $_.Name -match '^(LICENSE|COPYING|NOTICE|AUTHORS)' }
        if (-not $LicenseFiles) { throw "No license text found for $($Package.name); inspect before distribution" }
        foreach ($File in $LicenseFiles) {
            [void]$Notices.AppendLine("--- $($File.Name) ---")
            [void]$Notices.AppendLine([IO.File]::ReadAllText($File.FullName))
        }
    }
    [IO.File]::WriteAllText((Join-Path $DistRoot 'THIRD-PARTY-NOTICES.txt'), $Notices.ToString(), [Text.UTF8Encoding]::new($false))
    $BuildInfo = @{ product = 'UsbEject'; version = $ProductVersion; target = 'x86_64-pc-windows-msvc'; asset = $AssetName; sha256 = $Hash; builtUtc = [DateTime]::UtcNow.ToString('o'); guiAndHardwareValidated = $false }
    [IO.File]::WriteAllText((Join-Path $DistRoot 'build-info.json'), ($BuildInfo | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    & (Join-Path $PSScriptRoot 'inspect-package.ps1')
    Write-Output "Prepared $AssetPath (not launched or published)"
}
finally { Pop-Location }
