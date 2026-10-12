# LP2 Online installer. install.bat runs it; uninstall.bat runs it with -Uninstall.
param([switch]$Uninstall)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

function Test-Game($dir) { $dir -and ((Test-Path "$dir\LP2DX11.exe") -or (Test-Path "$dir\LP2DX9.exe")) }

function Find-Game {
    # 1. This folder or the one above it (the zip unpacked into the game folder).
    foreach ($d in @($here, (Split-Path $here -Parent))) { if (Test-Game $d) { return $d } }
    # 2. Any Steam library.
    $steam = (Get-ItemProperty 'HKCU:\Software\Valve\Steam' -ErrorAction SilentlyContinue).SteamPath
    if ($steam) {
        $libs = @($steam)
        $vdf = "$steam\steamapps\libraryfolders.vdf"
        if (Test-Path $vdf) { $libs += (Select-String -Path $vdf -Pattern '"path"\s+"([^"]+)"' -AllMatches).Matches | ForEach-Object { $_.Groups[1].Value -replace '\\', '\' } }
        foreach ($l in $libs) { $d = "$l\steamapps\common\Lost Planet 2"; if (Test-Game $d) { return $d } }
    }
    # 3. Ask (disc, retail and other copies live anywhere).
    Add-Type -AssemblyName System.Windows.Forms
    $dlg = New-Object System.Windows.Forms.FolderBrowserDialog
    $dlg.Description = 'Select your Lost Planet 2 folder (the one with LP2DX11.exe or LP2DX9.exe)'
    while ($dlg.ShowDialog() -eq 'OK') {
        if (Test-Game $dlg.SelectedPath) { return $dlg.SelectedPath }
        [System.Windows.Forms.MessageBox]::Show('That folder has no LP2DX11.exe or LP2DX9.exe.', 'LP2 Online') | Out-Null
    }
    throw 'No Lost Planet 2 folder selected.'
}

$game = Find-Game
Write-Host "Lost Planet 2 folder: $game"

if ($Uninstall) {
    if (Test-Path "$game\xlive.dll.bak") { Move-Item -Force "$game\xlive.dll.bak" "$game\xlive.dll" } else { Remove-Item -ErrorAction SilentlyContinue "$game\xlive.dll" }
    # Map and message archives the DLL patched for bots.
    Get-ChildItem "$game\nativePC" -Recurse -Filter '*.arc.orig' -ErrorAction SilentlyContinue |
        ForEach-Object { Move-Item -Force $_.FullName ($_.FullName -replace '\.orig$', '') }
    Write-Host 'LP2 Online removed; the game is back to how it was (your lp2*.ini files are kept).'
    exit 0
}

if ((Test-Path "$game\xlive.dll") -and -not (Test-Path "$game\xlive.dll.bak")) { Rename-Item "$game\xlive.dll" 'xlive.dll.bak' }
Copy-Item -Force "$here\xlive.dll" "$game\xlive.dll"
if (-not (Test-Path "$game\lp2mods.ini")) { Copy-Item "$here\lp2mods.ini.example" "$game\lp2mods.ini" }
if (-not (Test-Path "$game\lp2net.ini")) {
    $name = Read-Host "Your player name, up to 15 letters (Enter for $env:USERNAME)"
    if (-not $name) { $name = $env:USERNAME }
    $name = ($name -replace '[^\x20-\x7E]', '').Trim()
    if ($name.Length -gt 15) { $name = $name.Substring(0, 15) }
    if (-not $name) { $name = 'Player' }
    "name = $name`r`nlog = true`r`n" | Set-Content -Encoding ASCII "$game\lp2net.ini"
}
Write-Host 'Installed. Start Lost Planet 2, then Online > Player Match to find matches.'
