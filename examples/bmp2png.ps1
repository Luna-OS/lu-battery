# Converts the preview's BMP frame to PNG: bmp2png.ps1 in.bmp out.png
param([string]$In, [string]$Out)
Add-Type -AssemblyName System.Drawing
$img = [System.Drawing.Image]::FromFile($In)
$img.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$img.Dispose()
