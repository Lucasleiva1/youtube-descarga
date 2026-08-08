[CmdletBinding()]
param(
  [Parameter(Mandatory)]
  [ValidatePattern('^https?://')]
  [string] $Url,

  [ValidateRange(1, 4320)]
  [int] $ExpectedHeight = 1080
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$target = (& rustc --print host-tuple).Trim()
$binaryRoot = Join-Path $projectRoot 'src-tauri\binaries'
$ytDlp = Join-Path $binaryRoot "yt-dlp-$target.exe"
$ffmpeg = Join-Path $binaryRoot "ffmpeg-$target.exe"
$deno = Join-Path $binaryRoot "deno-$target.exe"

foreach ($required in @($ytDlp, $ffmpeg, $deno)) {
  if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
    throw "Falta el motor requerido: $required"
  }
}

$output = & $ytDlp `
  --ignore-config `
  --no-plugin-dirs `
  --dump-single-json `
  --skip-download `
  --no-warnings `
  --no-playlist `
  --ffmpeg-location $ffmpeg `
  --js-runtimes "deno:$deno" `
  -- $Url 2>&1

if ($LASTEXITCODE -ne 0) {
  throw "El análisis público falló: $($output -join [Environment]::NewLine)"
}

$metadata = ($output -join [Environment]::NewLine) | ConvertFrom-Json
$videoFormats = @($metadata.formats | Where-Object {
  $_.vcodec -and $_.vcodec -ne 'none' -and $_.url -and -not $_.has_drm
})
$heights = @($videoFormats | Where-Object height | Select-Object -ExpandProperty height -Unique | Sort-Object)

if (-not $metadata.id -or $videoFormats.Count -eq 0) {
  throw 'yt-dlp respondió sin un video o sin formatos descargables.'
}

if ($ExpectedHeight -notin $heights) {
  throw "No apareció la altura esperada de ${ExpectedHeight}p. Alturas: $($heights -join ', ')"
}

[pscustomobject]@{
  Id = $metadata.id
  Title = $metadata.title
  Availability = $metadata.availability
  Formats = $videoFormats.Count
  Heights = $heights -join ','
  ExpectedHeight = $ExpectedHeight
  Result = 'OK (sin descarga)'
} | Format-List
