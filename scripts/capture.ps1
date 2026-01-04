param([string]$OutputRoot = '', [switch]$Measure)
$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$Executable = Join-Path $RepoRoot 'target\x86_64-pc-windows-msvc\release\UsbEject.exe'
if (-not $OutputRoot) { $OutputRoot = Join-Path $RepoRoot ('.test-output\visual-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')) }
$OutputRoot = [IO.Path]::GetFullPath($OutputRoot)
New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null
function Invoke-BoundedDiagnostic([string[]]$Arguments, [int]$Timeout) {
    $StartInfo = [Diagnostics.ProcessStartInfo]::new($Executable)
    $StartInfo.WorkingDirectory = $RepoRoot
    $StartInfo.UseShellExecute = $false
    $StartInfo.CreateNoWindow = $true
    foreach ($Argument in $Arguments) { $StartInfo.ArgumentList.Add($Argument) }
    $Process = [Diagnostics.Process]::Start($StartInfo)
    $ProcessId = $Process.Id
    if (-not $Process.WaitForExit($Timeout)) {
        # Only this isolated, disposable diagnostic process is terminated on timeout.
        $Process.Kill($true)
        throw "Diagnostic process $ProcessId timed out"
    }
    if ($Process.ExitCode -ne 0) { throw "Diagnostic process $ProcessId failed: $($Process.ExitCode)" }
    $Process.Dispose()
}
foreach ($Screen in @('empty', 'sample', 'error', 'settings', 'details')) {
    $Path = Join-Path $OutputRoot "$Screen.png"
    if (Test-Path -LiteralPath $Path) { throw "Refusing to reuse a previous capture: $Path" }
    Invoke-BoundedDiagnostic @('--capture', $Path, '--screen', $Screen) 12000
    if (-not (Test-Path -LiteralPath $Path)) { throw "Capture was not created: $Path" }
    $CaptureMetadata = Get-Content -LiteralPath ([IO.Path]::ChangeExtension($Path, 'capture.json')) -Raw | ConvertFrom-Json
    foreach ($Case in @('under_state_borrow','outside_state_borrow')) {
        if ($CaptureMetadata.reentrant_palette.$Case | Where-Object { $_.background -ne 16777215 -or -not $_.brush_available }) { throw "Reentrant palette regression: $Screen / $Case" }
    }
}
$ScaledPath = Join-Path $OutputRoot 'sample-150.png'
Invoke-BoundedDiagnostic @('--capture', $ScaledPath, '--screen', 'sample', '--scale', '150') 12000
$IconPath = Join-Path $OutputRoot 'tray-icons.png'
Invoke-BoundedDiagnostic @('--capture', $IconPath, '--screen', 'icons') 12000
if (-not (Test-Path -LiteralPath $IconPath)) { throw 'Native icon proof was not created' }
if ($Measure) {
    $MeasurePath = Join-Path $OutputRoot 'rendering-load.json'
    Invoke-BoundedDiagnostic @('--measure', $MeasurePath) 30000
    if (-not (Test-Path -LiteralPath $MeasurePath)) { throw 'Measurement report was not created' }
    Get-Content -LiteralPath $MeasurePath -Raw
    $SettingsMeasurePath = Join-Path $OutputRoot 'settings-rendering-load.json'
    Invoke-BoundedDiagnostic @('--measure', $SettingsMeasurePath, '--screen', 'settings') 30000
    Get-Content -LiteralPath $SettingsMeasurePath -Raw
    foreach ($ReportPath in @($MeasurePath, $SettingsMeasurePath)) {
        $Report = Get-Content -LiteralPath $ReportPath -Raw | ConvertFrom-Json
        foreach ($Phase in $Report.phases) {
            if ($Phase.paints -ne 0 -or $Phase.layouts -ne 0 -or $Phase.owner_draws -ne 0 -or $Phase.print_client_requests -ne 0 -or $Phase.cpu_percent_one_core -gt 5) { throw "Rendering/load gate requires investigation: $ReportPath / $($Phase.state)" }
            if ($Phase.state -eq 'hidden_with_data_update' -and $Phase.native_checkbox_backdrops -ne 0) { throw "Hidden native control rendering: $ReportPath" }
        }
    }
}
Write-Output "Application-generated captures: $OutputRoot"
