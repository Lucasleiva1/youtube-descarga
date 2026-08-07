# Binarios redistribuidos

La app empaqueta binarios Windows descargados por `scripts/setup-binaries.ps1` y verificados con los SHA-256 publicados por cada release.

| Motor | Fuente | Licencia |
| --- | --- | --- |
| yt-dlp | https://github.com/yt-dlp/yt-dlp/releases | Unlicense; el ejecutable incluye avisos de terceros. |
| FFmpeg / ffprobe | https://www.gyan.dev/ffmpeg/builds/ | El build puede incluir componentes LGPL/GPL. Antes de distribuir comercialmente hay que incluir los avisos y cumplir las obligaciones del build elegido. |
| Deno | https://github.com/denoland/deno/releases | MIT. |

No se descargan binarios al iniciar la aplicación final: quedan incluidos como sidecars en el instalador generado por Tauri.
