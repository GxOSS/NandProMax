$ErrorActionPreference = "Stop"

function Die($Message) {
  Write-Error $Message
  exit 1
}

param(
  [string]$OutDir = ""
)

if ([string]::IsNullOrWhiteSpace($OutDir)) {
  $OutDir = (Get-Location).Path
}

$os = "windows"
$arch = ""
try {
  $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
} catch {
  $arch = $env:PROCESSOR_ARCHITECTURE.ToLowerInvariant()
}

switch ($arch) {
  "x64" { $arch = "x86_64" }
  "amd64" { $arch = "x86_64" }
  "arm64" { $arch = "arm64" }
  default { Die "unsupported architecture: $arch" }
}

$UrlWindowsX86_64 = $env:URL_WINDOWS_X86_64
$UrlWindowsArm64 = $env:URL_WINDOWS_ARM64

if ([string]::IsNullOrWhiteSpace($UrlWindowsX86_64)) {
  $UrlWindowsX86_64 = "https://ftdichip.com/wp-content/uploads/2025/03/CDM-v2.12.36.20-WHQL-Certified.zip"
}
if ([string]::IsNullOrWhiteSpace($UrlWindowsArm64)) {
  $UrlWindowsArm64 = "https://ftdichip.com/wp-content/uploads/2025/03/CDM-v2.12.36.20-for-ARM64-WHQL-Certified.zip"
}

$url = ""
switch ($arch) {
  "x86_64" { $url = $UrlWindowsX86_64 }
  "arm64" { $url = $UrlWindowsArm64 }
}

if ([string]::IsNullOrWhiteSpace($url)) {
  Die "no download URL configured for windows_$arch (set URL_WINDOWS_X86_64 / URL_WINDOWS_ARM64)"
}

$tmpRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("nandpromax-ftd2xx-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tmpRoot | Out-Null

try {
  $artifact = Join-Path $tmpRoot "artifact"

  if ($url.StartsWith("file://")) {
    $src = $url.Substring(7)
    if (!(Test-Path -LiteralPath $src)) { Die "file URL does not exist: $url" }
    Copy-Item -Force -LiteralPath $src -Destination $artifact
  } elseif ($url.StartsWith("http://") -or $url.StartsWith("https://")) {
    $headers = @{
      "User-Agent" = ($env:HTTP_USER_AGENT)
      "Referer" = ($env:HTTP_REFERER)
    }
    if ([string]::IsNullOrWhiteSpace($headers["User-Agent"])) { $headers["User-Agent"] = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0 Safari/537.36" }
    if ([string]::IsNullOrWhiteSpace($headers["Referer"])) { $headers["Referer"] = "https://ftdichip.com/" }

    try {
      Invoke-WebRequest -Uri $url -Headers $headers -OutFile $artifact -MaximumRedirection 10
    } catch {
      Die "download failed (possible 403): $url"
    }
  } else {
    if (!(Test-Path -LiteralPath $url)) { Die "URL is not http(s) and file does not exist: $url" }
    Copy-Item -Force -LiteralPath $url -Destination $artifact
  }

  $extractDir = Join-Path $tmpRoot "extract"
  New-Item -ItemType Directory -Path $extractDir | Out-Null

  $isZip = $false
  if ($url.ToLowerInvariant().EndsWith(".zip")) { $isZip = $true }

  if ($isZip) {
    Expand-Archive -Path $artifact -DestinationPath $extractDir -Force
  } else {
    $tar = Get-Command tar -ErrorAction SilentlyContinue
    if ($null -eq $tar) { Die "unsupported archive type (need .zip or tar in PATH): $url" }
    & tar -xf $artifact -C $extractDir | Out-Null
  }

  $dll = Get-ChildItem -Path $extractDir -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -ieq "ftd2xx.dll" -or $_.Name -ieq "libftd2xx.dll" } |
    Select-Object -First 1

  if ($null -eq $dll) {
    Die "downloaded artifact does not contain ftd2xx.dll / libftd2xx.dll"
  }

  New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
  $dest = Join-Path $OutDir $dll.Name
  Copy-Item -Force -LiteralPath $dll.FullName -Destination $dest

  Write-Host ("installed: " + $dest)
} finally {
  Remove-Item -Recurse -Force -LiteralPath $tmpRoot -ErrorAction SilentlyContinue
}
