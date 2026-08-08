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

## Acceso automático a YouTube

Cada enlace se analiza de forma independiente y siempre comienza con acceso público, sin leer sesiones del navegador. Si YouTube devuelve un desafío técnico compatible, la aplicación inicia su proveedor local de PO Token en `127.0.0.1` y reintenta. Las sesiones de Firefox, Edge, Chrome o Brave solo se consultan cuando YouTube confirma que el contenido requiere una cuenta.

La aplicación no cierra navegadores, no desactiva el cifrado de cookies de Windows y no ejecuta el motor con privilegios de administrador. Los intentos y su clasificación quedan registrados, sin URL, cookies ni rutas de perfiles, en el archivo rotativo `youtube-access.log` del directorio de logs de la aplicación.

## Pruebas

```powershell
npm.cmd run build
cargo test --manifest-path .\src-tauri\Cargo.toml --locked
powershell.exe -ExecutionPolicy Bypass -File .\scripts\smoke-youtube.ps1 -Url "https://www.youtube.com/watch?v=VIDEO_ID" -ExpectedHeight 1080
```

La prueba de humo solo analiza metadatos y formatos; no descarga el video.
