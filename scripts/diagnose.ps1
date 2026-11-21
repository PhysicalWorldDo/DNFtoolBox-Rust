[CmdletBinding()]
param(
    [string]$Workspace,
    [string]$Executable,
    [string]$RequestFile,
    [switch]$WaitForOperation,
    [switch]$Gui
)
$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $Workspace) { $Workspace = Join-Path $projectRoot '.work\acceptance\empty-workspace' }
if (-not $Executable) { $Executable = Join-Path $projectRoot 'target\x86_64-pc-windows-msvc\release\PhysicalWorldToolbox.exe' }
$Workspace = [System.IO.Path]::GetFullPath($Workspace)
New-Item -ItemType Directory -Force -Path $Workspace | Out-Null
$report = Join-Path $Workspace 'host-diagnosis.json'
$start = [System.Diagnostics.ProcessStartInfo]::new()
$start.FileName = [System.IO.Path]::GetFullPath($Executable)
$start.WorkingDirectory = $Workspace
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.ArgumentList.Add('--workspace')
$start.ArgumentList.Add($Workspace)
if (-not $Gui) {
    if ($RequestFile) {
        $start.ArgumentList.Add('--request-file')
        $start.ArgumentList.Add([System.IO.Path]::GetFullPath($RequestFile))
        if ($WaitForOperation) { $start.ArgumentList.Add('--wait') }
    } else { $start.ArgumentList.Add('--diagnose') }
    $start.ArgumentList.Add('--output')
    $start.ArgumentList.Add($report)
}
$process = [System.Diagnostics.Process]::Start($start)
if ($Gui) {
    $ready = Join-Path $Workspace 'cache\logs\startup.json'
    $deadline = [DateTime]::UtcNow.AddSeconds(35)
    do {
        if (Test-Path -LiteralPath $ready) {
            $proof = Get-Content -LiteralPath $ready -Raw | ConvertFrom-Json
            if ($proof.pid -eq $process.Id -and $proof.stage -eq 'ready' -and $proof.embeddedFrontend) {
                $proof | ConvertTo-Json -Depth 8
                $null = $process.CloseMainWindow()
                $null = $process.WaitForExit(10000)
                return
            }
        }
        if ($process.HasExited) { throw "The desktop process exited before app.ready ($($process.ExitCode))." }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    $null = $process.CloseMainWindow()
    throw 'The embedded WebView did not confirm app.ready; inspect cache/logs/host.jsonl.'
}
if (-not $process.WaitForExit(200000)) { $process.Kill(); throw 'The headless request timed out.' }
if ($process.ExitCode -ne 0) { if (Test-Path -LiteralPath $report) { Get-Content -LiteralPath $report -Raw }; throw "The request failed ($($process.ExitCode))." }
Get-Content -LiteralPath $report -Raw
