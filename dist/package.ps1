# Builds the release zip from the build outputs: .\dist\package.ps1 0.2.0  ->  target\LP2Online-v0.2.0.zip
param([Parameter(Mandatory)][string]$Version)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$out = "$root\target\LP2Online-v$Version"
Remove-Item -Recurse -Force $out, "$out.zip" -ErrorAction SilentlyContinue
New-Item -ItemType Directory $out | Out-Null
Copy-Item "$root\target\i686-pc-windows-msvc\release\xlive.dll", "$root\target\release\lp2-master.exe", `
    "$root\lp2mods.ini.example", "$root\lp2net.ini.example", "$root\LICENSE", "$root\README.md", `
    "$PSScriptRoot\install.bat", "$PSScriptRoot\uninstall.bat", "$PSScriptRoot\install.ps1", `
    "$PSScriptRoot\host-server.bat", "$PSScriptRoot\START HERE.txt" $out
Compress-Archive -Path $out -DestinationPath "$out.zip"
Write-Host "$out.zip"
