$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$DistRoot = Join-Path $RepoRoot 'dist'
$BuildInfo = Get-Content -LiteralPath (Join-Path $DistRoot 'build-info.json') -Raw | ConvertFrom-Json
$ExecutablePath = Join-Path $DistRoot $BuildInfo.asset
$ImageBytes = [IO.File]::ReadAllBytes($ExecutablePath)
function Read-U16([int]$Position) { [BitConverter]::ToUInt16($ImageBytes, $Position) }
function Read-U32([int]$Position) { [BitConverter]::ToUInt32($ImageBytes, $Position) }
if ($ImageBytes.Length -lt 512 -or (Read-U16 0) -ne 0x5A4D) { throw 'Missing DOS header' }
$PeStart = Read-U32 0x3c
if ((Read-U32 $PeStart) -ne 0x4550 -or (Read-U16 ($PeStart + 4)) -ne 0x8664) { throw 'Package is not a Windows x64 PE image' }
$OptionalStart = $PeStart + 24
if ((Read-U16 $OptionalStart) -ne 0x20b -or (Read-U16 ($OptionalStart + 68)) -ne 2) { throw 'Expected PE32+ Windows GUI subsystem' }
$SectionCount = Read-U16 ($PeStart + 6)
$SectionStart = $OptionalStart + (Read-U16 ($PeStart + 20))
function Resolve-Rva([uint32]$Address) {
    for ($Index = 0; $Index -lt $SectionCount; $Index++) {
        $Section = $SectionStart + 40 * $Index
        $VirtualStart = Read-U32 ($Section + 12)
        $RawSize = Read-U32 ($Section + 16)
        if ($Address -ge $VirtualStart -and ($Address - $VirtualStart) -lt $RawSize) {
            return [int]((Read-U32 ($Section + 20)) + $Address - $VirtualStart)
        }
    }
    throw "Unmapped PE RVA $Address"
}
function Read-AsciiZ([int]$Position) {
    $End = $Position
    while ($End -lt $ImageBytes.Length -and $ImageBytes[$End] -ne 0 -and $End - $Position -lt 512) { $End++ }
    if ($End -ge $ImageBytes.Length -or $End - $Position -ge 512) { throw 'Invalid import name' }
    [Text.Encoding]::ASCII.GetString($ImageBytes, $Position, $End - $Position)
}
$Imports = [Collections.Generic.List[string]]::new()
$ImportRva = Read-U32 ($OptionalStart + 120)
if ($ImportRva -ne 0) {
    $ImportStart = Resolve-Rva $ImportRva
    for ($Index = 0; $Index -lt 256; $Index++) {
        $Descriptor = $ImportStart + 20 * $Index
        $NameRva = Read-U32 ($Descriptor + 12)
        if ($NameRva -eq 0) { break }
        $Imports.Add((Read-AsciiZ (Resolve-Rva $NameRva)))
    }
}
if ($Imports | Where-Object { $_ -match '(?i)^(vcruntime|msvcp|ucrtbase|api-ms-win-crt|libssl|libcrypto)' }) { throw 'Unexpected dynamic CRT or external TLS DLL dependency' }
$ResourceRoot = Resolve-Rva (Read-U32 ($OptionalStart + 128))
function Resource-Entry([int]$Directory, [int]$Id) {
    $Count = (Read-U16 ($Directory + 12)) + (Read-U16 ($Directory + 14))
    for ($Index = 0; $Index -lt $Count; $Index++) {
        $Entry = $Directory + 16 + 8 * $Index
        if ((Read-U32 $Entry) -eq $Id) { return (Read-U32 ($Entry + 4)) }
    }
    throw "Missing resource ID $Id"
}
$ManifestType = Resource-Entry $ResourceRoot 24
$ManifestName = Resource-Entry ($ResourceRoot + ($ManifestType -band 0x7fffffff)) 1
$LanguageDirectory = $ResourceRoot + ($ManifestName -band 0x7fffffff)
$ManifestData = Read-U32 ($LanguageDirectory + 20)
$DataEntry = $ResourceRoot + $ManifestData
$ManifestStart = Resolve-Rva (Read-U32 $DataEntry)
$ManifestSize = Read-U32 ($DataEntry + 4)
[xml]$Manifest = [Text.Encoding]::UTF8.GetString($ImageBytes, $ManifestStart, $ManifestSize).Trim([char]0)
$RequestedLevel = $Manifest.SelectSingleNode("//*[local-name()='requestedExecutionLevel']").level
$DpiAwareness = $Manifest.SelectSingleNode("//*[local-name()='dpiAwareness']").InnerText
if ($RequestedLevel -ne 'asInvoker' -or $DpiAwareness -ne 'PerMonitorV2') { throw 'Incorrect execution or DPI manifest' }
[void](Resource-Entry $ResourceRoot 14)
$VersionInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo($ExecutablePath)
if ($VersionInfo.ProductName -ne 'UsbEject' -or $VersionInfo.FileVersion -ne $BuildInfo.version -or $VersionInfo.OriginalFilename -ne $BuildInfo.asset) { throw 'Executable version resources do not match packaging metadata' }
$ActualHash = (Get-FileHash -LiteralPath $ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
$ExpectedLine = "$ActualHash  $($BuildInfo.asset)"
if ((Get-Content -LiteralPath (Join-Path $DistRoot 'SHA256SUMS') -Raw).Trim() -ne $ExpectedLine -or $ActualHash -ne $BuildInfo.sha256) { throw 'Packaged checksum mismatch' }
$Report = @{ asset = $BuildInfo.asset; bytes = $ImageBytes.Length; version = $VersionInfo.FileVersion; machine = 'x64'; subsystem = 'Windows GUI'; executionLevel = $RequestedLevel; dpiAwareness = $DpiAwareness; sha256 = $ActualHash; importedDlls = @($Imports | Sort-Object); launched = $false }
$Json = $Report | ConvertTo-Json -Depth 4
[IO.File]::WriteAllText((Join-Path $DistRoot 'inspection.json'), $Json, [Text.UTF8Encoding]::new($false))
Write-Output $Json
