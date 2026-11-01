param([string]$InputDirectory = '.release-input', [string]$OutputDirectory = '.release-stage')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Set-Location -LiteralPath $repository
$configuration = Get-Content -LiteralPath 'scripts/localization-release.json' -Raw | ConvertFrom-Json
$version = [regex]::Match((Get-Content -LiteralPath 'Cargo.toml' -Raw), '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$').Groups[1].Value
if (-not $version) { throw 'Expected a stable Cargo package version.' }
$expectedSubject = "Release localization v$version"
$event = Get-Content -LiteralPath $env:GITHUB_EVENT_PATH -Raw | ConvertFrom-Json
$subject = ($event.head_commit.message -split "`n")[0].TrimEnd("`r")
if ($env:GITHUB_EVENT_NAME -ne 'push' -or $env:GITHUB_REF -ne 'refs/heads/main' -or $subject -cne $expectedSubject) {
    throw "Publication requires an explicit main-push commit subject: $expectedSubject"
}
$notesPath = Join-Path $repository $configuration.notes
$notes = Get-Content -LiteralPath $notesPath -Raw -Encoding utf8
if ($notes.Length -lt 100 -or -not $notes.Contains($version)) { throw 'Release notes must describe this version in readable English.' }
$inputRoot = (Resolve-Path -LiteralPath $InputDirectory).Path
$outputRoot = [IO.Path]::GetFullPath((Join-Path $repository $OutputDirectory))
if (-not $outputRoot.StartsWith($repository + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The staging path must stay inside this repository.'
}
if (Test-Path -LiteralPath $outputRoot) { throw 'Use a fresh release staging directory.' }
New-Item -ItemType Directory -Path $outputRoot | Out-Null
$artifactFiles = @(Get-ChildItem -LiteralPath $inputRoot -Recurse -File)
$assetNames = @("$($configuration.product)-v$version.exe")
foreach ($asset in $configuration.additionalAssets) { $assetNames += $asset.Replace('{version}', $version) }
foreach ($name in $assetNames) {
    if ([IO.Path]::GetFileName($name) -cne $name) { throw 'Asset names must be simple filenames.' }
    $matches = @($artifactFiles | Where-Object Name -CEQ $name)
    if ($matches.Count -ne 1) { throw "Expected exactly one checked artifact named $name; got $($matches.Count)." }
    $source = $matches[0].FullName
    if ($name.EndsWith('.exe', [StringComparison]::OrdinalIgnoreCase)) {
        $bytes = [IO.File]::ReadAllBytes($source)
        if ($bytes.Length -lt 256 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) { throw 'Expected a Windows PE executable.' }
        $pe = [BitConverter]::ToInt32($bytes, 0x3c)
        if ($pe -lt 0 -or $pe + 6 -gt $bytes.Length -or [BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550 -or [BitConverter]::ToUInt16($bytes, $pe + 4) -ne 0x8664) {
            throw 'Expected a Windows x64 executable.'
        }
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $outputRoot $name)
}

# Keep license texts from the final sources and generated package together.
$noticeRoot = Join-Path $outputRoot 'notices'
New-Item -ItemType Directory -Path $noticeRoot | Out-Null
$tracked = @(& git ls-files)
if ($LASTEXITCODE -ne 0) { throw 'Cannot list license files from the checked commit.' }
$licenseFiles = @($tracked | Where-Object { $_ -match '(?i)(^|/)(LICENSE[^/]*|COPYING[^/]*|NOTICE[^/]*|THIRD[-_]?PARTY[^/]*|DEPENDENCY_LICENSES[^/]*)(/|$)|(^|/)licenses/' })
if (-not @($licenseFiles | Where-Object { $_ -match '^LICENSE(?:\.[^/]+)?$' }).Count) { throw 'The root project license is missing.' }
foreach ($relative in $licenseFiles) {
    $source = Join-Path $repository $relative
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { continue }
    $destination = Join-Path $noticeRoot (Join-Path 'source' $relative)
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination
}
foreach ($file in $artifactFiles | Where-Object { $_.Name -match '(?i)license|copying|notice|third[-_]?party' }) {
    $relative = [IO.Path]::GetRelativePath($inputRoot, $file.FullName)
    $destination = Join-Path $noticeRoot (Join-Path 'package' $relative)
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
    Copy-Item -LiteralPath $file.FullName -Destination $destination
}
$noticeName = "$($configuration.product)-v$version-notices.zip"
Compress-Archive -Path (Join-Path $noticeRoot '*') -DestinationPath (Join-Path $outputRoot $noticeName)
$assetNames += $noticeName
$manifest = @()
$checksumLines = @()
foreach ($name in $assetNames) {
    $assetPath = Join-Path $outputRoot $name
    $digest = (Get-FileHash -LiteralPath $assetPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $manifest += [ordered]@{name=$name; size=(Get-Item -LiteralPath $assetPath).Length; digest="sha256:$digest"}
    $checksumLines += "$digest  $name"
}
[IO.File]::WriteAllText((Join-Path $outputRoot 'SHA256SUMS.txt'), ($checksumLines -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
$checksumPath = Join-Path $outputRoot 'SHA256SUMS.txt'
$manifest += [ordered]@{name='SHA256SUMS.txt'; size=(Get-Item -LiteralPath $checksumPath).Length; digest=('sha256:' + (Get-FileHash -LiteralPath $checksumPath -Algorithm SHA256).Hash.ToLowerInvariant())}
$checksumName = 'SHA256SUMS.txt'
if ($configuration.PSObject.Properties['checksumName']) {
    $checksumName = $configuration.checksumName.Replace('{executable}', "$($configuration.product)-v$version.exe")
}
if ([IO.Path]::GetFileName($checksumName) -cne $checksumName) { throw 'The updater checksum contract must be a simple filename.' }
if ($checksumName -cne 'SHA256SUMS.txt') {
    $compatibilityPath = Join-Path $outputRoot $checksumName
    if ($checksumName.EndsWith('.sha256', [StringComparison]::OrdinalIgnoreCase)) {
        [IO.File]::WriteAllText($compatibilityPath, $checksumLines[0] + "`n", [Text.UTF8Encoding]::new($false))
    } else {
        Copy-Item -LiteralPath $checksumPath -Destination $compatibilityPath
    }
    $manifest += [ordered]@{name=$checksumName; size=(Get-Item -LiteralPath $compatibilityPath).Length; digest=('sha256:' + (Get-FileHash -LiteralPath $compatibilityPath -Algorithm SHA256).Hash.ToLowerInvariant())}
}
$release = [ordered]@{version=$version; tag="v$version"; product=$configuration.product; commit=$env:GITHUB_SHA; notes=$configuration.notes; assets=@($manifest)}
[IO.File]::WriteAllText((Join-Path $outputRoot 'manifest.json'), ($release | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
Write-Output "Staged $($manifest.Count) checked release assets for v$version."
