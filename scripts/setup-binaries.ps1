[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$binaryDirectory = Join-Path $projectRoot 'src-tauri\binaries'
$target = (& rustc --print host-tuple).Trim()

if ($target -ne 'x86_64-pc-windows-msvc') {
  throw "El setup todavía no dispone de assets para '$target'. Agregá los assets de ese target antes de compilar."
}

$temporaryDirectory = Join-Path ([System.IO.Path]::GetTempPath()) ('yt-download-binaries-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $binaryDirectory -Force | Out-Null
New-Item -ItemType Directory -Path $temporaryDirectory -Force | Out-Null

function Get-VerifiedFile {
  param(
    [Parameter(Mandatory)] [string] $Url,
    [Parameter(Mandatory)] [string] $ChecksumUrl,
    [Parameter(Mandatory)] [string] $Destination,
    [Parameter(Mandatory)] [string] $AssetName
  )
  $checksumResponse = Invoke-WebRequest -UseBasicParsing -Uri $ChecksumUrl
  $checksumText = if ($checksumResponse.Content -is [byte[]]) { [Text.Encoding]::UTF8.GetString($checksumResponse.Content) } else { [string] $checksumResponse.Content }
  $assetPattern = [regex]::Escape($AssetName)
  if ($checksumText -notmatch "(?mi)^([a-f0-9]{64})\s+\*?.*$assetPattern\s*$") {
    if ($checksumText -notmatch '(?i)([a-f0-9]{64})') { throw "No se encontró SHA-256 para $AssetName." }
  }
  $expected = $matches[1].ToLowerInvariant()
  Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Destination
  $actual = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $expected) {
    Remove-Item -LiteralPath $Destination -Force
    throw "Checksum inválido para $AssetName. Esperado: $expected; recibido: $actual"
  }
}

try {
  $ytDlp = Join-Path $binaryDirectory "yt-dlp-$target.exe"
  if (-not (Test-Path $ytDlp)) {
    Get-VerifiedFile -Url 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe' -ChecksumUrl 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS' -Destination $ytDlp -AssetName 'yt-dlp.exe'
  }

  $denoArchive = Join-Path $temporaryDirectory 'deno.zip'
  $denoAsset = 'deno-x86_64-pc-windows-msvc.zip'
  $deno = Join-Path $binaryDirectory "deno-$target.exe"
  if (-not (Test-Path $deno)) {
    Get-VerifiedFile -Url "https://github.com/denoland/deno/releases/latest/download/$denoAsset" -ChecksumUrl "https://github.com/denoland/deno/releases/latest/download/$denoAsset.sha256sum" -Destination $denoArchive -AssetName $denoAsset
    Expand-Archive -LiteralPath $denoArchive -DestinationPath (Join-Path $temporaryDirectory 'deno') -Force
    Copy-Item -LiteralPath (Join-Path $temporaryDirectory 'deno\deno.exe') -Destination $deno -Force
  }

  $ffmpegArchive = Join-Path $temporaryDirectory 'ffmpeg.zip'
  Get-VerifiedFile -Url 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip' -ChecksumUrl 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256' -Destination $ffmpegArchive -AssetName 'ffmpeg-release-essentials.zip'
  Expand-Archive -LiteralPath $ffmpegArchive -DestinationPath (Join-Path $temporaryDirectory 'ffmpeg') -Force
  $ffmpeg = Get-ChildItem -Path (Join-Path $temporaryDirectory 'ffmpeg') -Filter 'ffmpeg.exe' -Recurse | Select-Object -First 1
  $ffprobe = Get-ChildItem -Path (Join-Path $temporaryDirectory 'ffmpeg') -Filter 'ffprobe.exe' -Recurse | Select-Object -First 1
  if (-not $ffmpeg -or -not $ffprobe) { throw 'El archivo de FFmpeg no contiene ffmpeg.exe y ffprobe.exe.' }
  Copy-Item -LiteralPath $ffmpeg.FullName -Destination (Join-Path $binaryDirectory "ffmpeg-$target.exe") -Force
  Copy-Item -LiteralPath $ffprobe.FullName -Destination (Join-Path $binaryDirectory "ffprobe-$target.exe") -Force

  & $ytDlp --version
  & (Join-Path $binaryDirectory "ffmpeg-$target.exe") -version | Select-Object -First 1
  & (Join-Path $binaryDirectory "ffprobe-$target.exe") -version | Select-Object -First 1
  & $deno --version | Select-Object -First 1
}
finally {
  if (Test-Path $temporaryDirectory) { Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force }
}
