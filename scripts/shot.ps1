# Capture the true composited Atlas window (acrylic/glass shows only via CopyFromScreen,
# never PrintWindow — PrintWindow renders the transparent window over black).
param([string]$Out = "$env:TEMP\atlas-shot.png", [string]$Proc = "atlas")

Add-Type -AssemblyName System.Drawing
$sig = @'
using System;
using System.Runtime.InteropServices;
public class Win {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  public struct RECT { public int Left, Top, Right, Bottom; }
}
'@
Add-Type $sig

$p = Get-Process $Proc -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $p) { Write-Error "No window for process '$Proc'"; exit 1 }
$h = $p.MainWindowHandle
[Win]::ShowWindow($h, 3) | Out-Null            # SW_MAXIMIZE
$HWND_TOPMOST = New-Object IntPtr(-1)
[Win]::SetWindowPos($h, $HWND_TOPMOST, 0,0,0,0, 0x0003) | Out-Null  # NOSIZE|NOMOVE
[Win]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 400

$r = New-Object Win+RECT
[Win]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size($w, $ht)))
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$HWND_NOTOPMOST = New-Object IntPtr(-2)
[Win]::SetWindowPos($h, $HWND_NOTOPMOST, 0,0,0,0, 0x0003) | Out-Null
Write-Output $Out
