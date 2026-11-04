param([switch]$SkipBuild)

$ErrorActionPreference = "Stop"
$repo = Split-Path $PSScriptRoot -Parent
$output = Join-Path $repo "target\extensions\cloud"
$version = "v1.74.4"
$sha256 = "ef097ef9de37a57feb7d9f9c7afb34148ad3c65be8025f1d8f7f521554a701ea"
$asset = "rclone-$version-windows-amd64.zip"
$zip = Join-Path $repo "target\extensions\$asset"

if (-not $SkipBuild) {
    Push-Location $repo
    try {
        cargo build --release --locked -p atlas-cloud-extension
        if ($LASTEXITCODE -ne 0) { throw "Cloud extension build failed ($LASTEXITCODE)." }
    } finally {
        Pop-Location
    }
}

$binary = Join-Path $repo "target\release\atlas-cloud-extension.exe"
if (-not (Test-Path $binary)) {
    throw "Missing $binary. Build the Cloud extension first."
}

New-Item -ItemType Directory -Force -Path (Split-Path $zip) | Out-Null
if (-not (Test-Path $zip) -or (Get-FileHash $zip -Algorithm SHA256).Hash -ne $sha256) {
    Invoke-WebRequest "https://downloads.rclone.org/$version/$asset" -OutFile $zip
}
if ((Get-FileHash $zip -Algorithm SHA256).Hash -ne $sha256) {
    throw "rclone $version SHA-256 mismatch."
}

$unpack = Join-Path $repo "target\extensions\rclone"
foreach ($directory in @($unpack, $output)) {
    $resolved = [System.IO.Path]::GetFullPath($directory)
    $allowed = [System.IO.Path]::GetFullPath((Join-Path $repo "target\extensions")) + [System.IO.Path]::DirectorySeparatorChar
    if (-not $resolved.StartsWith($allowed, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to clean a directory outside target/extensions: $resolved"
    }
}
Remove-Item -LiteralPath $unpack -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $output -Recurse -Force -ErrorAction SilentlyContinue
Expand-Archive -LiteralPath $zip -DestinationPath $unpack
New-Item -ItemType Directory -Force -Path $output | Out-Null
Copy-Item $binary (Join-Path $output "atlas-cloud-extension.exe")
Copy-Item (Join-Path $repo "extensions\cloud\extension.toml") $output
Copy-Item (Join-Path $repo "extensions\cloud\icon.svg") $output
Copy-Item (Join-Path $repo "extensions\cloud\THIRD-PARTY-NOTICES.txt") $output
$rclone = Get-ChildItem $unpack -Filter rclone.exe -Recurse |
    Select-Object -First 1 -ExpandProperty FullName
Copy-Item $rclone (Join-Path $output "rclone.exe")

$archive = "$output.zip"
Compress-Archive -Path (Join-Path $output "*") -DestinationPath $archive -Force
Write-Output "Installable Cloud package: $archive"

Get-ChildItem $output
