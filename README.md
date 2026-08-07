# YT Download

Aplicación de escritorio Windows para analizar y descargar contenido autorizado de YouTube de forma local.

## Tecnología

- Tauri v2 + Rust
- React + TypeScript + Vite + Tailwind
- yt-dlp, FFmpeg/ffprobe y Deno como sidecars

## Desarrollo

```powershell
npm.cmd install
powershell.exe -ExecutionPolicy Bypass -File .\scripts\setup-binaries.ps1
npm.cmd run tauri dev
```

Los sidecars Windows se versionan con Git LFS para preservar una versión reproducible del motor de desarrollo.
