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

Cada enlace se analiza de forma independiente y siempre comienza con acceso público, sin leer sesiones del navegador. Si YouTube devuelve un desafío técnico compatible, la aplicación inicia su proveedor local de PO Token en `127.0.0.1` y reintenta una sola vez. Si el contenido exige una cuenta, la aplicación se detiene: las sesiones de Firefox, Edge, Chrome o Brave no se consultan nunca.

La aplicación no cierra navegadores, no desactiva el cifrado de cookies de Windows y no ejecuta el motor con privilegios de administrador. Los intentos y su clasificación quedan registrados, sin URL, cookies ni rutas de perfiles, en el archivo rotativo `youtube-access.log` del directorio de logs de la aplicación.

Para reducir bloqueos por exceso de solicitudes, todas las extracciones y descargas comparten un único turno de red y dejan al menos ocho segundos entre operaciones. El análisis expone las resoluciones originales disponibles, elige la mayor por defecto y excluye variantes `-sr` de Super Resolution. No prueba todas las fuentes durante el análisis: comprueba solamente la seleccionada al iniciar la descarga, fuerza IPv4 y, si esa fuente falla, reintenta una vez con otra fuente de la misma resolución sin bajar la calidad silenciosamente. También espera entre las solicitudes internas de extracción, reutiliza durante 30 minutos los resultados ya obtenidos y detecta enlaces repetidos por ID antes de acceder a Internet. Si el historial apunta a un archivo que todavía existe, la interfaz permite abrirlo sin volver a consultar YouTube.

Cuando YouTube devuelve un límite explícito o rechaza tanto el acceso público como el proveedor PO, la aplicación activa un enfriamiento local de 30 minutos. Ese estado se guarda en los datos de la aplicación y sobrevive a un reinicio; los clics repetidos durante ese período no generan tráfico nuevo. Si la aplicación continúa abierta, los análisis bloqueados y los trabajos que ya estaban en cola se reanudan automáticamente al terminar la pausa, con un único reintento automático para evitar ciclos infinitos. No se usan proxies rotativos, resolución automática de CAPTCHA ni cambios de huella del dispositivo.

## Pruebas

```powershell
npm.cmd run build
cargo test --manifest-path .\src-tauri\Cargo.toml --locked
powershell.exe -ExecutionPolicy Bypass -File .\scripts\smoke-youtube.ps1 -Url "https://www.youtube.com/watch?v=VIDEO_ID" -ExpectedHeight 1080
```

La prueba de humo solo analiza metadatos y formatos; no descarga el video.
