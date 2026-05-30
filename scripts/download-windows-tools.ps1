param(
    [string]$OutputDir = "$PSScriptRoot\..\src-tauri\resources"
)

$ErrorActionPreference = "Stop"

$OutputDir = Resolve-Path $OutputDir -ErrorAction SilentlyContinue
if (-not $OutputDir) {
    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
    $OutputDir = Resolve-Path $OutputDir
}

Write-Host "Downloading Windows tools to: $OutputDir"

# ── ADB (platform-tools) ──
Write-Host "`n[1/2] Downloading ADB (platform-tools)..."
$adbZip = "$env:TEMP\platform-tools-latest-windows.zip"
Invoke-WebRequest -Uri "https://dl.google.com/android/repository/platform-tools-latest-windows.zip" -OutFile $adbZip -UseBasicParsing

Write-Host "  Extracting adb.exe, AdbWinApi.dll, AdbWinUsbApi.dll..."
$adbTemp = "$env:TEMP\platform-tools"
if (Test-Path $adbTemp) { Remove-Item -Recurse -Force $adbTemp }
Expand-Archive -Path $adbZip -DestinationPath $adbTemp -Force

Copy-Item "$adbTemp\platform-tools\adb.exe" "$OutputDir\adb.exe" -Force
Copy-Item "$adbTemp\platform-tools\AdbWinApi.dll" "$OutputDir\AdbWinApi.dll" -Force
Copy-Item "$adbTemp\platform-tools\AdbWinUsbApi.dll" "$OutputDir\AdbWinUsbApi.dll" -Force

Remove-Item -Recurse -Force $adbTemp

# ── scrcpy ──
Write-Host "`n[2/2] Downloading scrcpy..."
$latestRelease = Invoke-RestMethod -Uri "https://api.github.com/repos/Genymobile/scrcpy/releases/latest" -UseBasicParsing
$version = $latestRelease.tag_name -replace '^v'
$scrcpyUrl = "https://github.com/Genymobile/scrcpy/releases/download/v$version/scrcpy-win64-v$version.zip"
Write-Host "  Version: v$version"

$scrcpyZip = "$env:TEMP\scrcpy-win64-v$version.zip"
Invoke-WebRequest -Uri $scrcpyUrl -OutFile $scrcpyZip -UseBasicParsing

Write-Host "  Extracting all files..."
$scrcpyTemp = "$env:TEMP\scrcpy-win64"
if (Test-Path $scrcpyTemp) { Remove-Item -Recurse -Force $scrcpyTemp }
Expand-Archive -Path $scrcpyZip -DestinationPath $scrcpyTemp -Force

# Copy everything from the extracted folder to resources
$extractedDir = Get-ChildItem "$scrcpyTemp\scrcpy-win64*" | Select-Object -First 1
if ($extractedDir) {
    Copy-Item "$($extractedDir.FullName)\*" "$OutputDir\" -Recurse -Force
} else {
    Copy-Item "$scrcpyTemp\*" "$OutputDir\" -Recurse -Force
}

Remove-Item -Recurse -Force $scrcpyTemp

Write-Host "`nDone! Files in: $OutputDir"
Get-ChildItem $OutputDir | Select-Object Name, Length | Format-Table -AutoSize
