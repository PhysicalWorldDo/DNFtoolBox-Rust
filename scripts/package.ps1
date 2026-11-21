[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
& (Join-Path $PSScriptRoot 'build.ps1')

$application = Get-Content -LiteralPath (Join-Path $projectRoot 'src-tauri/tauri.conf.json') -Raw -Encoding utf8 | ConvertFrom-Json
$fileName = 'PhysicalWorldToolbox.exe'
$compiled = Join-Path $projectRoot "target/x86_64-pc-windows-msvc/release/$fileName"
$distribution = New-Item -ItemType Directory -Path (Join-Path $projectRoot 'dist') -Force
$portable = Join-Path $distribution.FullName $fileName
$archive = Join-Path $distribution.FullName ("PhysicalWorldToolbox-{0}-win-x64.zip" -f $application.version)
Copy-Item -LiteralPath $compiled -Destination $portable -Force
Compress-Archive -LiteralPath $portable -DestinationPath $archive -CompressionLevel Optimal -Force
Get-FileHash -LiteralPath $portable,$archive -Algorithm SHA256 | Format-Table -AutoSize
