# Regenerate icon.ico from 1024x1024.png with full Windows DPI size set.
#
# Why this script exists:
#   tauri-icon emits an .ico containing only 16/32/48/64/128/256, which leaves
#   Windows shell (taskbar, title bar, explorer) with no native size for
#   24 / 30 / 36 / 40 / 60 / 72 / 96 / 142 / 150 -- the sizes Windows actually
#   asks for at 100% / 125% / 150% / 175% / 200% DPI. The shell then bilinear-
#   downscales the closest available size, producing visible blur.
#
# This script bakes a full DPI matrix (16/20/24/32/40/48/64/96/128/256) using
# HighQualityBicubic resampling, then packs them as PNG-encoded entries into a
# single .ico. Windows 10/11 read PNG-encoded ICO entries natively.
#
# Usage (from anywhere):
#   powershell -ExecutionPolicy Bypass -File backend/crates/golish/icons/regen-icon-ico.ps1
#
# After running, restart the application and clear the Windows icon cache
# (commands printed at the end) so the taskbar picks up the new icon.

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing

$IconDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Source  = Join-Path $IconDir '1024x1024.png'
$Out     = Join-Path $IconDir 'icon.ico'
$Backup  = Join-Path $IconDir 'icon.ico.bak'

if (-not (Test-Path $Source)) {
    Write-Error "Source PNG not found: $Source"
    exit 1
}

# Sizes covering Windows DPI scaling for taskbar (24/30/36/42/48), title bar
# (16/20/24/28/32), explorer small (16/20/24/28/32) / medium (32/40/48/56/64),
# desktop default (48/60/72/84/96), and large icon view (128/256).
$sizes = @(16, 20, 24, 32, 40, 48, 64, 96, 128, 256)

Write-Host "Loading source: $Source"
$src = [System.Drawing.Image]::FromFile($Source)
try {
    if ($src.Width -lt 1024 -or $src.Height -lt 1024) {
        Write-Warning "Source is only $($src.Width)x$($src.Height); 1024x1024+ recommended for clean small sizes."
    }

    $pngBytes = New-Object 'System.Collections.Generic.List[byte[]]'

    foreach ($s in $sizes) {
        Write-Host ("Rendering {0}x{0}" -f $s)
        $bmp = New-Object System.Drawing.Bitmap($s, $s, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $g = [System.Drawing.Graphics]::FromImage($bmp)
            try {
                $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
                $g.InterpolationMode  = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $g.SmoothingMode      = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                $g.PixelOffsetMode    = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                # Transparent background (the source PNG already has alpha; this is
                # belt-and-suspenders so partial-edge pixels don't pick up white).
                $g.Clear([System.Drawing.Color]::Transparent)
                $rect = New-Object System.Drawing.Rectangle 0, 0, $s, $s
                $g.DrawImage($src, $rect)
            } finally {
                $g.Dispose()
            }

            $ms = New-Object System.IO.MemoryStream
            try {
                $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
                $pngBytes.Add($ms.ToArray())
            } finally {
                $ms.Dispose()
            }
        } finally {
            $bmp.Dispose()
        }
    }
} finally {
    $src.Dispose()
}

# ---- Pack ICO -----------------------------------------------------------
# ICONDIR (6 bytes): reserved(2)=0, type(2)=1 (icon), count(2)
# ICONDIRENTRY (16 bytes per image): width(1), height(1), colorCount(1)=0,
#   reserved(1)=0, planes(2)=1, bitCount(2)=32, sizeInBytes(4), offset(4)
# Then each image's raw PNG bytes back-to-back.
$count = $sizes.Count
$headerSize  = 6 + 16 * $count
$totalSize   = $headerSize + ($pngBytes | Measure-Object -Property Length -Sum).Sum

$ico = New-Object byte[] $totalSize
$pos = 0

# ICONDIR
$ico[$pos] = 0; $ico[$pos + 1] = 0      # reserved
$ico[$pos + 2] = 1; $ico[$pos + 3] = 0  # type = 1 (icon)
$ico[$pos + 4] = [byte]($count -band 0xFF)
$ico[$pos + 5] = [byte](($count -shr 8) -band 0xFF)
$pos += 6

# ICONDIRENTRY[]
$dataOffset = $headerSize
for ($i = 0; $i -lt $count; $i++) {
    $w = $sizes[$i]
    $h = $sizes[$i]
    $size = $pngBytes[$i].Length

    $ico[$pos]     = if ($w -ge 256) { 0 } else { [byte]$w }   # 0 means 256
    $ico[$pos + 1] = if ($h -ge 256) { 0 } else { [byte]$h }
    $ico[$pos + 2] = 0   # color count
    $ico[$pos + 3] = 0   # reserved
    $ico[$pos + 4] = 1; $ico[$pos + 5] = 0  # planes = 1
    $ico[$pos + 6] = 32; $ico[$pos + 7] = 0 # bitCount = 32

    $ico[$pos + 8]  = [byte]( $size        -band 0xFF)
    $ico[$pos + 9]  = [byte](($size -shr 8)  -band 0xFF)
    $ico[$pos + 10] = [byte](($size -shr 16) -band 0xFF)
    $ico[$pos + 11] = [byte](($size -shr 24) -band 0xFF)

    $ico[$pos + 12] = [byte]( $dataOffset        -band 0xFF)
    $ico[$pos + 13] = [byte](($dataOffset -shr 8)  -band 0xFF)
    $ico[$pos + 14] = [byte](($dataOffset -shr 16) -band 0xFF)
    $ico[$pos + 15] = [byte](($dataOffset -shr 24) -band 0xFF)

    $pos += 16
    $dataOffset += $size
}

# Image data
for ($i = 0; $i -lt $count; $i++) {
    [Array]::Copy($pngBytes[$i], 0, $ico, $pos, $pngBytes[$i].Length)
    $pos += $pngBytes[$i].Length
}

# Write with backup
if (Test-Path $Out) {
    Copy-Item -LiteralPath $Out -Destination $Backup -Force
    Write-Host "Backed up old icon to: $Backup"
}
[System.IO.File]::WriteAllBytes($Out, $ico)
Write-Host "Wrote: $Out  (size = $($ico.Length) bytes, $count images)"

Write-Host ""
Write-Host "Verifying packed ICO..."
$bytes = [System.IO.File]::ReadAllBytes($Out)
$packedCount = [BitConverter]::ToUInt16($bytes, 4)
Write-Host "  embedded images: $packedCount"
for ($i = 0; $i -lt $packedCount; $i++) {
    $off = 6 + $i * 16
    $w = $bytes[$off]; $h = $bytes[$off + 1]
    $bpp = [BitConverter]::ToUInt16($bytes, $off + 6)
    $sz  = [BitConverter]::ToUInt32($bytes, $off + 8)
    $imgOff = [BitConverter]::ToUInt32($bytes, $off + 12)
    $wA = if ($w -eq 0) { 256 } else { $w }
    $hA = if ($h -eq 0) { 256 } else { $h }
    $isPng = ($bytes[$imgOff] -eq 0x89 -and $bytes[$imgOff + 1] -eq 0x50)
    $fmt = if ($isPng) { 'PNG' } else { 'BMP' }
    Write-Host ("  [{0}] {1}x{2}  {3}bpp  {4}  size={5}" -f $i, $wA, $hA, $bpp, $fmt, $sz)
}

Write-Host ""
Write-Host "Done. To make Windows pick up the new icon, clear the icon cache:"
Write-Host '  taskkill /im explorer.exe /f'
Write-Host '  del /A /F /Q "$env:LocalAppData\Microsoft\Windows\Explorer\iconcache*"'
Write-Host '  del /A /F /Q "$env:LocalAppData\Microsoft\Windows\Explorer\thumbcache*"'
Write-Host '  start explorer.exe'
Write-Host 'Then rebuild and restart the Tauri app.'
