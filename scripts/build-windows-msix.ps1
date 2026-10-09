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
$unplatedIcon = Join-Path $repo "packaging\icons\atlas-unplated.png"
if (-not (Test-Path $unplatedIcon)) {
    $unplatedIcon = $icon
}

Stop-Process -Name "atlas", "atlas-search-service", "atlas-cloud-extension", "atlas-icon-extension" -Force -ErrorAction SilentlyContinue

if (-not $SkipBuild) {
    Push-Location $repo
    try {
        cargo build --release --locked -p atlas -p atlas-search-service -p atlas-cloud-extension -p atlas-icon-extension
    } finally {
        Pop-Location
    }
}

$exe = Join-Path $repo "target\release\atlas.exe"
if (-not (Test-Path $exe)) {
    throw "Missing $exe. Build Atlas first or omit -SkipBuild."
}

# Update extension binaries in user AppData if existing
$cloudExtTarget = Join-Path $env:APPDATA "atlas\extensions\cloud\atlas-cloud-extension.exe"
$cloudExtBuilt = Join-Path $repo "target\release\atlas-cloud-extension.exe"
if ((Test-Path (Split-Path $cloudExtTarget)) -and (Test-Path $cloudExtBuilt)) {
    Copy-Item -LiteralPath $cloudExtBuilt -Destination $cloudExtTarget -Force
}
$iconExtTarget = Join-Path $env:APPDATA "atlas\extensions\icon-manager\atlas-icon-extension.exe"
$iconExtBuilt = Join-Path $repo "target\release\atlas-icon-extension.exe"
if ((Test-Path (Split-Path $iconExtTarget)) -and (Test-Path $iconExtBuilt)) {
    Copy-Item -LiteralPath $iconExtBuilt -Destination $iconExtTarget -Force
}

Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
$assets = New-Item -ItemType Directory -Force -Path (Join-Path $staging "Assets")
Copy-Item -LiteralPath $exe -Destination (Join-Path $staging "atlas.exe")
$serviceExe = Join-Path $repo "target\release\atlas-search-service.exe"
if (Test-Path $serviceExe) {
    Copy-Item -LiteralPath $serviceExe -Destination (Join-Path $staging "atlas-search-service.exe")
}
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

function Save-ResizedAsset {
    param(
        [System.Drawing.Image]$SourceImage,
        [int]$TargetWidth,
        [int]$TargetHeight,
        [string]$DestinationPath,
        [switch]$Contain
    )
    $bitmap = [System.Drawing.Bitmap]::new($TargetWidth, $TargetHeight)
    try {
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.Clear([System.Drawing.Color]::Transparent)
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
            $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality

            if ($Contain) {
                $scale = [Math]::Min($TargetWidth / $SourceImage.Width, $TargetHeight / $SourceImage.Height)
                $destW = [int]($SourceImage.Width * $scale)
                $destH = [int]($SourceImage.Height * $scale)
                $destX = [int](($TargetWidth - $destW) / 2)
                $destY = [int](($TargetHeight - $destH) / 2)
                $graphics.DrawImage($SourceImage, $destX, $destY, $destW, $destH)
            } else {
                $graphics.DrawImage($SourceImage, 0, 0, $TargetWidth, $TargetHeight)
            }
        } finally {
            $graphics.Dispose()
        }
        $bitmap.Save($DestinationPath, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $bitmap.Dispose()
    }
}

$source = [System.Drawing.Image]::FromFile($icon)
$sourceUnplated = [System.Drawing.Image]::FromFile($unplatedIcon)
try {
    # 1. Square150x150Logo (Medium Tile)
    $sq150Scales = @{
        "Square150x150Logo.png"           = @(150, 150)
        "Square150x150Logo.scale-100.png" = @(150, 150)
        "Square150x150Logo.scale-125.png" = @(188, 188)
        "Square150x150Logo.scale-150.png" = @(225, 225)
        "Square150x150Logo.scale-200.png" = @(300, 300)
        "Square150x150Logo.scale-400.png" = @(600, 600)
    }
    foreach ($k in $sq150Scales.Keys) {
        $w, $h = $sq150Scales[$k]
        Save-ResizedAsset -SourceImage $source -TargetWidth $w -TargetHeight $h -DestinationPath (Join-Path $assets.FullName $k)
    }

    # 2. StoreLogo
    $storeScales = @{
        "StoreLogo.png"           = @(50, 50)
        "StoreLogo.scale-100.png" = @(50, 50)
        "StoreLogo.scale-125.png" = @(63, 63)
        "StoreLogo.scale-150.png" = @(75, 75)
        "StoreLogo.scale-200.png" = @(100, 100)
        "StoreLogo.scale-400.png" = @(200, 200)
    }
    foreach ($k in $storeScales.Keys) {
        $w, $h = $storeScales[$k]
        Save-ResizedAsset -SourceImage $source -TargetWidth $w -TargetHeight $h -DestinationPath (Join-Path $assets.FullName $k)
    }

    # 3. Wide310x150Logo
    $wideScales = @{
        "Wide310x150Logo.png"           = @(310, 150)
        "Wide310x150Logo.scale-100.png" = @(310, 150)
        "Wide310x150Logo.scale-125.png" = @(388, 188)
        "Wide310x150Logo.scale-150.png" = @(465, 225)
        "Wide310x150Logo.scale-200.png" = @(620, 300)
        "Wide310x150Logo.scale-400.png" = @(1240, 600)
    }
    foreach ($k in $wideScales.Keys) {
        $w, $h = $wideScales[$k]
        Save-ResizedAsset -SourceImage $source -TargetWidth $w -TargetHeight $h -DestinationPath (Join-Path $assets.FullName $k) -Contain
    }

    # 4. Square44x44Logo (App List & System scales)
    $sq44Scales = @{
        "Square44x44Logo.png"               = @(44, 44)
        "Square44x44Logo.scale-100.png"     = @(44, 44)
        "Square44x44Logo.scale-125.png"     = @(55, 55)
        "Square44x44Logo.scale-150.png"     = @(66, 66)
        "Square44x44Logo.scale-200.png"     = @(88, 88)
        "Square44x44Logo.scale-400.png"     = @(176, 176)
        "Square44x44Logo.targetsize-16.png"  = @(16, 16)
        "Square44x44Logo.targetsize-24.png"  = @(24, 24)
        "Square44x44Logo.targetsize-32.png"  = @(32, 32)
        "Square44x44Logo.targetsize-48.png"  = @(48, 48)
        "Square44x44Logo.targetsize-256.png" = @(256, 256)
    }
    foreach ($k in $sq44Scales.Keys) {
        $w, $h = $sq44Scales[$k]
        Save-ResizedAsset -SourceImage $source -TargetWidth $w -TargetHeight $h -DestinationPath (Join-Path $assets.FullName $k)
    }

    # 5. Taskbar Unplated variants (Crucial for modern Windows 10/11 taskbar!)
    $unplatedSizes = @(16, 24, 32, 48, 256)
    foreach ($sz in $unplatedSizes) {
        Save-ResizedAsset -SourceImage $sourceUnplated -TargetWidth $sz -TargetHeight $sz `
            -DestinationPath (Join-Path $assets.FullName "Square44x44Logo.altform-unplated_targetsize-$sz.png")
        Save-ResizedAsset -SourceImage $sourceUnplated -TargetWidth $sz -TargetHeight $sz `
            -DestinationPath (Join-Path $assets.FullName "Square44x44Logo.altform-lightunplated_targetsize-$sz.png")
    }
} finally {
    $source.Dispose()
    $sourceUnplated.Dispose()
}

$makeAppx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" `
    -Filter makeappx.exe -Recurse |
    Where-Object FullName -Match "\\x64\\makeappx\.exe$" |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if (-not $makeAppx) {
    throw "MakeAppx.exe not found. Install the Windows 10/11 SDK."
}

$makepri = Join-Path (Split-Path $makeAppx -Parent) "makepri.exe"
if (Test-Path $makepri) {
    $priconfig = Join-Path $target "priconfig.xml"
    & $makepri createconfig /cf $priconfig /dq lang-en-US /pv 10.0.0 /o | Out-Null
    & $makepri new /pr $staging /cf $priconfig /of (Join-Path $staging "resources.pri") /o | Out-Null
    Remove-Item -LiteralPath $priconfig -Force -ErrorAction SilentlyContinue
}

$package = Join-Path $target "Atlas_${Version}_${Architecture}.msix"
& $makeAppx pack /o /d $staging /p $package
if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx failed with exit code $LASTEXITCODE."
}

$signtool = Join-Path (Split-Path $makeAppx -Parent) "signtool.exe"
if (Test-Path $signtool) {
    try {
        $cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq "CN=49A3C9B5-859D-4613-AE50-D104622E4EBC" } | Select-Object -First 1
        if (-not $cert) {
            $cert = New-SelfSignedCertificate -Type Custom -Subject "CN=49A3C9B5-859D-4613-AE50-D104622E4EBC" `
                -KeyUsage DigitalSignature -FriendlyName "Atlas Dev Cert" -CertStoreLocation "Cert:\CurrentUser\My" `
                -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
        }
        $store = [System.Security.Cryptography.X509Certificates.X509Store]::new("TrustedPeople", "CurrentUser")
        $store.Open("ReadWrite")
        $store.Add($cert)
        $store.Close()
        & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint /q $package
        Write-Host "Successfully signed $package with dev cert ($($cert.Thumbprint))." -ForegroundColor Green
    } catch {
        Write-Warning "Failed to sign MSIX package: $_"
    }
}

Get-Item $package
Get-FileHash $package -Algorithm SHA256
