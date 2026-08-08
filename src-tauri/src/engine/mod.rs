use std::{fs, path::{Path, PathBuf}};
use tauri::Manager;

#[derive(Debug, Clone)]
pub struct EnginePaths {
    pub yt_dlp: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    pub deno: Option<PathBuf>,
}

impl EnginePaths {
    pub fn resolve(app: &tauri::AppHandle) -> Self {
        Self {
            yt_dlp: resolve_binary(app, "yt-dlp"),
            ffmpeg: resolve_binary(app, "ffmpeg"),
            ffprobe: resolve_binary(app, "ffprobe"),
            deno: resolve_binary(app, "deno"),
        }
    }

    pub fn required_for_youtube(&self) -> Result<(), String> {
        let missing: Vec<&str> = [
            ("yt-dlp", self.yt_dlp.is_none()),
            ("ffmpeg", self.ffmpeg.is_none()),
            ("ffprobe", self.ffprobe.is_none()),
            ("deno", self.deno.is_none()),
        ]
        .into_iter()
        .filter_map(|(name, missing)| missing.then_some(name))
        .collect();
        if missing.is_empty() { Ok(()) } else { Err(format!("Motor incompleto: falta {}.", missing.join(", "))) }
    }
}

pub fn resolve_binary(app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    let extension = if cfg!(windows) { ".exe" } else { "" };
    let triple = option_env!("TAURI_ENV_TARGET_TRIPLE").unwrap_or("x86_64-pc-windows-msvc");
    let sidecar_name = format!("{name}-{triple}{extension}");
    let source_binary = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(&sidecar_name);
    let resources = app.path().resource_dir().ok();
    let executable_dir = std::env::current_exe().ok().and_then(|path| path.parent().map(PathBuf::from));
    let mut candidates = vec![Some(source_binary)];
    if let Some(resource_dir) = resources {
        candidates.push(Some(resource_dir.join("binaries").join(&sidecar_name)));
        candidates.push(Some(resource_dir.join(&sidecar_name)));
    }
    if let Some(executable_dir) = executable_dir {
        candidates.push(Some(executable_dir.join(&sidecar_name)));
        candidates.push(Some(executable_dir.join("resources").join(&sidecar_name)));
    }
    candidates.into_iter().flatten().find(|path| path.is_file())
}

fn link_or_copy(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.is_file() && destination.metadata().ok().map(|metadata| metadata.len()) == source.metadata().ok().map(|metadata| metadata.len()) {
        return Ok(());
    }
    if destination.exists() { fs::remove_file(destination).map_err(|error| format!("No se pudo actualizar FFmpeg: {error}"))?; }
    if fs::hard_link(source, destination).is_err() {
        fs::copy(source, destination).map_err(|error| format!("No se pudo preparar FFmpeg: {error}"))?;
    }
    Ok(())
}

/// yt-dlp expects executables literally named ffmpeg/ffprobe. Tauri sidecars
/// must carry the target triple, so expose private aliases in app data instead
/// of relying on a global PATH.
pub fn yt_dlp_ffmpeg_location(app: &tauri::AppHandle, paths: &EnginePaths) -> Result<PathBuf, String> {
    let ffmpeg = paths.ffmpeg.as_ref().ok_or_else(|| "FFmpeg no está disponible.".to_owned())?;
    let ffprobe = paths.ffprobe.as_ref().ok_or_else(|| "ffprobe no está disponible.".to_owned())?;
    let directory = app.path().app_data_dir().map_err(|error| format!("No se pudo resolver el directorio del motor: {error}"))?.join("engine-tools");
    fs::create_dir_all(&directory).map_err(|error| format!("No se pudo crear el directorio del motor: {error}"))?;
    let extension = if cfg!(windows) { ".exe" } else { "" };
    link_or_copy(ffmpeg, &directory.join(format!("ffmpeg{extension}")))?;
    link_or_copy(ffprobe, &directory.join(format!("ffprobe{extension}")))?;
    Ok(directory)
}
