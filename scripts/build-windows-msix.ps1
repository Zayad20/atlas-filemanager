param(
    [ValidatePattern('^\d+\.\d+\.\d+\.\d+$')]
    [string]$Version = "0.1.0.0",
    [ValidateSet("x64")]
    [string]$Architecture = "x64",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repo = Split-Path $PSScriptRoot -Parent
$target = Join-Path $repo "target\msix"
$staging = Join-Path $target "staging"
$manifest = Join-Path $repo "packaging\windows\msix\Package.appxmanifest"
$icon = Join-Path $repo "packaging\icons\atlas.png"

if (-not $SkipBuild) {
    Push-Location $repo
    try {
        cargo build --release --locked
    } finally {
        Pop-Location
    }
}

$exe = Join-Path $repo "target\release\atlas.exe"
if (-not (Test-Path $exe)) {
    throw "Missing $exe. Build Atlas first or omit -SkipBuild."
}

Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
$assets = New-Item -ItemType Directory -Force -Path (Join-Path $staging "Assets")
Copy-Item -LiteralPath $exe -Destination (Join-Path $staging "atlas.exe")
Copy-Item -LiteralPath (Join-Path $repo "crates\atlas-core\vendor\unrar-bin\UnRAR.exe") `
    -Destination (Join-Path $staging "UnRAR.exe")
Copy-Item -LiteralPath (Join-Path $repo "packaging\windows\msix\THIRD-PARTY-NOTICES.txt") `
    -Destination $staging


[xml]$xml = Get-Content -Raw $manifest
$identity = $xml.Package.Identity
$identity.Version = $Version
$identity.ProcessorArchitecture = $Architecture
$xml.Save((Join-Path $staging "AppxManifest.xml"))

Add-Type -AssemblyName System.Drawing
$source = [System.Drawing.Image]::FromFile($icon)
try {
    $sizes = @{
        "StoreLogo.png" = @(50, 50)
        "Square44x44Logo.png" = @(44, 44)
        "Square150x150Logo.png" = @(150, 150)
        "Wide310x150Logo.png" = @(310, 150)
    }
    foreach ($name in $sizes.Keys) {
        $width, $height = $sizes[$name]
        $bitmap = [System.Drawing.Bitmap]::new($width, $height)
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.Clear([System.Drawing.Color]::Transparent)
                $graphics.InterpolationMode =
                    [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $side = [Math]::Min($width, $height)
                $graphics.DrawImage($source, (($width - $side) / 2), 0, $side, $side)
            } finally {
                $graphics.Dispose()
            }
            $bitmap.Save(
                (Join-Path $assets.FullName $name),
                [System.Drawing.Imaging.ImageFormat]::Png
            )
        } finally {
            $bitmap.Dispose()
        }
    }
} finally {
    $source.Dispose()
}

$makeAppx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" `
    -Filter makeappx.exe -Recurse |
    Where-Object FullName -Match "\\x64\\makeappx\.exe$" |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if (-not $makeAppx) {
    throw "MakeAppx.exe not found. Install the Windows 10/11 SDK."
}

$package = Join-Path $target "Atlas_${Version}_${Architecture}.msix"
& $makeAppx pack /o /d $staging /p $package
if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx failed with exit code $LASTEXITCODE."
}

Get-Item $package
Get-FileHash $package -Algorithm SHA256
