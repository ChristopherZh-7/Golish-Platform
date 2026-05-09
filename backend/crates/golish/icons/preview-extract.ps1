# Extract every embedded PNG from icon.ico into icons/.preview/ so you can
# eyeball the scaling quality at the exact sizes Windows shell will request.

$ErrorActionPreference = 'Stop'

$IconDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$IcoPath = Join-Path $IconDir 'icon.ico'
$PreviewDir = Join-Path $IconDir '.preview'

if (-not (Test-Path $IcoPath)) { Write-Error "icon.ico not found"; exit 1 }
if (-not (Test-Path $PreviewDir)) { New-Item -ItemType Directory -Path $PreviewDir | Out-Null }

$bytes = [System.IO.File]::ReadAllBytes($IcoPath)
$count = [BitConverter]::ToUInt16($bytes, 4)

for ($i = 0; $i -lt $count; $i++) {
    $off = 6 + $i * 16
    $w = $bytes[$off]; $h = $bytes[$off + 1]
    $size = [BitConverter]::ToUInt32($bytes, $off + 8)
    $imgOff = [BitConverter]::ToUInt32($bytes, $off + 12)
    $wA = if ($w -eq 0) { 256 } else { $w }
    $hA = if ($h -eq 0) { 256 } else { $h }

    $isPng = ($bytes[$imgOff] -eq 0x89 -and $bytes[$imgOff + 1] -eq 0x50)
    if (-not $isPng) { continue }

    $slice = New-Object byte[] $size
    [Array]::Copy($bytes, $imgOff, $slice, 0, $size)
    $out = Join-Path $PreviewDir ("ico_{0:00}_{1}x{2}.png" -f $i, $wA, $hA)
    [System.IO.File]::WriteAllBytes($out, $slice)
    Write-Host "wrote $out"
}
