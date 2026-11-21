[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$buildRoot = Join-Path $projectRoot '.work\build'
$temporaryRoot = Join-Path $buildRoot 'temp'
New-Item -ItemType Directory -Force -Path $temporaryRoot | Out-Null
$savedTemp = $env:TEMP
$savedTmp = $env:TMP
try {
    $env:TEMP = $temporaryRoot
    $env:TMP = $temporaryRoot
    if (-not (Get-Command rc.exe -ErrorAction SilentlyContinue)) {
        $vswherePath = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        if (-not (Test-Path -LiteralPath $vswherePath)) { throw 'Visual Studio Build Tools and Windows SDK are required.' }
        $installation = & $vswherePath -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if (-not $installation) { throw 'No MSVC x64 build tools installation was found.' }
        $developerShell = Join-Path $installation 'Common7\Tools\Launch-VsDevShell.ps1'
        & $developerShell -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
    }
    if (-not (Get-Command rc.exe -ErrorAction SilentlyContinue)) { throw 'Windows SDK resource compiler RC.EXE is unavailable.' }
    Push-Location -LiteralPath $projectRoot
    try {
        & cargo +stable-x86_64-pc-windows-msvc build --release --locked --target x86_64-pc-windows-msvc -p physical-world-toolbox --features custom-protocol 2>&1 | Tee-Object -FilePath (Join-Path $buildRoot 'release.log')
        if ($LASTEXITCODE -ne 0) { throw "Cargo release build failed ($LASTEXITCODE)." }
        $executable = Join-Path $projectRoot 'target\x86_64-pc-windows-msvc\release\PhysicalWorldToolbox.exe'
        if (-not (Test-Path -LiteralPath $executable)) { throw 'The expected release EXE was not produced.' }
        $dependencies = & dumpbin.exe /DEPENDENTS $executable 2>&1
        $dependencies | Set-Content -LiteralPath (Join-Path $buildRoot 'pe-dependents.txt') -Encoding utf8
        if ($LASTEXITCODE -ne 0) { throw 'PE dependency inspection failed.' }
        if (($dependencies -join "`n") -match '(?i)(WebView2Loader|python\d*|Qt[56]\w*|VCRUNTIME\w*|MSVCP\w*|libgcc\w*|libstdc\w*|libwinpthread\w*)\.dll') { throw 'The EXE depends on a private runtime DLL. Inspect .work/build/pe-dependents.txt.' }
        Get-FileHash -LiteralPath $executable -Algorithm SHA256 | Format-List
        Write-Output $executable
    } finally { Pop-Location }
} finally {
    $env:TEMP = $savedTemp
    $env:TMP = $savedTmp
}
