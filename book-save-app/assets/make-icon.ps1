param(
    [string]$Output = (Join-Path $PSScriptRoot 'book-save.ico')
)

$ErrorActionPreference = 'Stop'

# Generates assets/book-save.ico: a clean "BS" monogram in PNG-compressed
# ICO entries (16, 32, 48, 256 px). No runtime or package dependencies;
# uses only the .NET System.Drawing available on Windows.

Add-Type -AssemblyName System.Drawing

$sizes = @(16, 32, 48, 256)
$background = [System.Drawing.Color]::FromArgb(255, 31, 58, 95)   # deep slate blue: visible on light and dark taskbars
$foreground = [System.Drawing.Color]::FromArgb(255, 255, 255, 255) # white monogram
$borderColor = [System.Drawing.Color]::FromArgb(110, 255, 255, 255)

function New-RoundedPath($size, $radius) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $diameter = $radius * 2
    $path.AddArc(0, 0, $diameter, $diameter, 180, 90) | Out-Null
    $path.AddArc($size - $diameter - 1, 0, $diameter, $diameter, 270, 90) | Out-Null
    $path.AddArc($size - $diameter - 1, $size - $diameter - 1, $diameter, $diameter, 0, 90) | Out-Null
    $path.AddArc(0, $size - $diameter - 1, $diameter, $diameter, 90, 90) | Out-Null
    $path.CloseFigure() | Out-Null
    return $path
}

$pngBlobs = @()
foreach ($size in $sizes) {
    $bitmap = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
            $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
            $graphics.Clear([System.Drawing.Color]::Transparent)
            $radius = [Math]::Max(2, [int]($size * 0.22))
            $path = New-RoundedPath $size $radius
            try {
                $brush = New-Object System.Drawing.SolidBrush($background)
                try {
                    $graphics.FillPath($brush, $path) | Out-Null
                } finally { $brush.Dispose() }
                $penWidth = [Math]::Max(1, $size / 64)
                $pen = New-Object System.Drawing.Pen($borderColor, $penWidth)
                try {
                    $graphics.DrawPath($pen, $path) | Out-Null
                } finally { $pen.Dispose() }
            } finally { $path.Dispose() }

            # Fit "BS" into the tile with side padding so it stays legible at 16 px.
            $format = New-Object System.Drawing.StringFormat
            $format.Alignment = [System.Drawing.StringAlignment]::Center
            $format.LineAlignment = [System.Drawing.StringAlignment]::Center
            $fontSize = $size * 0.5
            $font = $null
            for ($attempt = 0; $attempt -lt 4; $attempt++) {
                if ($font) { $font.Dispose() }
                $font = New-Object System.Drawing.Font('Segoe UI', $fontSize, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
                $measured = $graphics.MeasureString('BS', $font)
                $targetW = $size * 0.72
                $targetH = $size * 0.56
                $scale = [Math]::Min($targetW / $measured.Width, $targetH / $measured.Height)
                if ([Math]::Abs($scale - 1.0) -lt 0.02) { break }
                $fontSize = $fontSize * $scale
            }
            try {
                $textBrush = New-Object System.Drawing.SolidBrush($foreground)
                try {
                    $rect = New-Object System.Drawing.RectangleF -ArgumentList @([float]0, [float]($size * 0.01), [float]$size, [float]$size)
                    $graphics.DrawString('BS', $font, $textBrush, $rect, $format) | Out-Null
                } finally { $textBrush.Dispose() }
            } finally { $font.Dispose(); $format.Dispose() }
        } finally { $graphics.Dispose() }

        $stream = New-Object System.IO.MemoryStream
        try {
            $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
            $pngBlobs += @(, $stream.ToArray())
        } finally { $stream.Dispose() }
    } finally { $bitmap.Dispose() }
}

# ICO container with PNG-compressed entries (Vista and later, incl. Win10/11).
$entryCount = $pngBlobs.Count
$headerSize = 6 + 16 * $entryCount
$offset = $headerSize
$stream = New-Object System.IO.MemoryStream
try {
    $writer = New-Object System.IO.BinaryWriter($stream)
    try {
        $writer.Write([uint16]0)          # reserved
        $writer.Write([uint16]1)          # type: icon
        $writer.Write([uint16]$entryCount)
        for ($i = 0; $i -lt $entryCount; $i++) {
            $size = $sizes[$i]
            $bytes = $pngBlobs[$i]
            $byte = if ($size -ge 256) { 0 } else { $size }
            $writer.Write([byte]$byte)    # width (0 = 256)
            $writer.Write([byte]$byte)    # height
            $writer.Write([byte]0)        # palette
            $writer.Write([byte]0)        # reserved
            $writer.Write([uint16]1)      # planes
            $writer.Write([uint16]32)     # bit count
            $writer.Write([uint32]$bytes.Length)
            $writer.Write([uint32]$offset)
            $offset += $bytes.Length
        }
        foreach ($bytes in $pngBlobs) {
            $writer.Write($bytes)
        }
        $writer.Flush()
    } finally { $writer.Dispose() }
    New-Item -ItemType Directory -Path (Split-Path $Output) -Force | Out-Null
    [System.IO.File]::WriteAllBytes($Output, $stream.ToArray())
} finally { $stream.Dispose() }

$length = (Get-Item -LiteralPath $Output).Length
Write-Output "Wrote $Output ($length bytes, entries: $($sizes -join ', ') px)"
