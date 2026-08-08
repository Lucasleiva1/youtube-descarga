use crate::hidden_command;
use std::{fs, path::{Path, PathBuf}};
use tauri::Manager;

const POT_PROVIDER_VERSION: &str = "1.3.1";
const POT_PLUGIN_FILE: &str = "bgutil-ytdlp-pot-provider.zip";

/// Local paths used by yt-dlp's proof-of-origin token provider. The provider
/// runs as a short lived child process; it never starts a public web server,
/// and all its cache files stay within the application's own data directory.
#[derive(Debug, Clone)]
pub struct PotProviderPaths {
    pub plugin_directory: PathBuf,
    pub server_home: PathBuf,
    pub deno_directory: PathBuf,
    pub cache_home: PathBuf,
}

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
    // Tauri keeps the target-triple name in the source tree, but bundles the
    // Windows sidecars with their final executable names next to the app.
    // Check both forms so an installed build never depends on this repository.
    let bundled_name = format!("{name}{extension}");
    let source_binary = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(&sidecar_name);
    let resources = app.path().resource_dir().ok();
    let executable_dir = std::env::current_exe().ok().and_then(|path| path.parent().map(PathBuf::from));
    let mut candidates = vec![Some(source_binary)];
    if let Some(resource_dir) = resources {
        candidates.push(Some(resource_dir.join("binaries").join(&sidecar_name)));
        candidates.push(Some(resource_dir.join(&sidecar_name)));
        candidates.push(Some(resource_dir.join("binaries").join(&bundled_name)));
        candidates.push(Some(resource_dir.join(&bundled_name)));
    }
    if let Some(executable_dir) = executable_dir {
        candidates.push(Some(executable_dir.join(&sidecar_name)));
        candidates.push(Some(executable_dir.join("resources").join(&sidecar_name)));
        candidates.push(Some(executable_dir.join(&bundled_name)));
        candidates.push(Some(executable_dir.join("resources").join(&bundled_name)));
    }
    candidates.into_iter().flatten().find(|path| path.is_file())
}

fn resource_directory(app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join(name);
    let mut candidates = vec![source];
    if let Ok(resources) = app.path().resource_dir() {
        candidates.push(resources.join(name));
        candidates.push(resources.join("resources").join(name));
    }
    candidates.into_iter().find(|path| path.is_dir())
}

fn copy_resource_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| format!("No se pudo preparar los recursos locales: {error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("No se pudo leer los recursos locales: {error}"))? {
        let entry = entry.map_err(|error| format!("No se pudo leer los recursos locales: {error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| format!("No se pudo inspeccionar los recursos locales: {error}"))?;
        if file_type.is_dir() {
            copy_resource_tree(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| format!("No se pudo preparar los recursos locales: {error}"))?;
        }
    }
    Ok(())
}

fn copy_resource_file(source: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination.parent().ok_or_else(|| "No se pudo preparar los recursos locales.".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| format!("No se pudo preparar los recursos locales: {error}"))?;
    fs::copy(source, destination).map_err(|error| format!("No se pudo preparar los recursos locales: {error}"))?;
    Ok(())
}

fn provider_environment(command: &mut std::process::Command, provider: &PotProviderPaths) {
    command
        .env("DENO_DIR", &provider.deno_directory)
        .env("XDG_CACHE_HOME", &provider.cache_home)
        .env("DENO_NO_PROMPT", "1")
        .env("DENO_NO_UPDATE_CHECK", "1");
}

fn provider_script_check(deno: &Path, provider: &PotProviderPaths) -> Result<(), String> {
    let node_modules = provider.server_home.join("node_modules");
    let token_cache = provider.cache_home.join("bgutil-ytdlp-pot-provider");
    let read_paths = format!("{},{}", token_cache.display(), node_modules.display());
    let mut command = hidden_command(deno);
    command
        .current_dir(&provider.server_home)
        .args(["run", "--allow-env", "--allow-net"])
        .arg(format!("--allow-ffi={}", node_modules.display()))
        .arg(format!("--allow-write={}", token_cache.display()))
        .arg(format!("--allow-read={read_paths}"))
        .arg(provider.server_home.join("src").join("generate_once.ts"))
        .arg("--version");
    provider_environment(&mut command, provider);
    let output = command.output().map_err(|_| "No se pudo iniciar el verificador local de YouTube.".to_owned())?;
    if output.status.success() { Ok(()) } else { Err("No se pudo preparar el verificador local de YouTube. Comprobá la conexión y reintentá.".to_owned()) }
}

/// Prepare the provider only after YouTube rejects the normal public access.
/// Dependencies are pinned by the vendor lockfile and cached locally on the
/// first use, so subsequent analyses run without a visible installer window.
pub fn ensure_pot_provider(app: &tauri::AppHandle, paths: &EnginePaths) -> Result<PotProviderPaths, String> {
    let deno = paths.deno.as_ref().ok_or_else(|| "Deno no está disponible.".to_owned())?;
    let bundled_provider = resource_directory(app, "pot-provider")
        .ok_or_else(|| "No se encontró el verificador local incluido.".to_owned())?;
    let bundled_plugins = resource_directory(app, "yt-dlp-plugins")
        .ok_or_else(|| "No se encontró el complemento local incluido.".to_owned())?;
    let app_data = app.path().app_data_dir().map_err(|_| "No se pudo resolver el directorio de la aplicación.".to_owned())?;
    let root = app_data.join(format!("youtube-pot-provider-{POT_PROVIDER_VERSION}"));
    let server_home = root.join("server");
    let plugin_directory = root.join("plugins");
    let provider = PotProviderPaths {
        plugin_directory: plugin_directory.clone(),
        server_home: server_home.clone(),
        deno_directory: root.join("deno-cache"),
        cache_home: root.join("cache"),
    };
    let ready_marker = root.join("ready");

    copy_resource_tree(&bundled_provider.join("server"), &server_home)?;
    copy_resource_file(&bundled_plugins.join(POT_PLUGIN_FILE), &plugin_directory.join(POT_PLUGIN_FILE))?;

    if !ready_marker.is_file() {
        let mut install = hidden_command(deno);
        install
            .current_dir(&server_home)
            .args(["install", "--allow-scripts=npm:canvas", "--frozen"]);
        provider_environment(&mut install, &provider);
        let output = install.output().map_err(|_| "No se pudo iniciar la preparación local de YouTube.".to_owned())?;
        if !output.status.success() {
            return Err("No se pudo preparar el verificador local de YouTube. Comprobá la conexión y reintentá.".to_owned());
        }
        provider_script_check(deno, &provider)?;
        fs::write(&ready_marker, POT_PROVIDER_VERSION).map_err(|_| "No se pudo completar la preparación local de YouTube.".to_owned())?;
    }

    Ok(provider)
}

/// Configures only the application-bundled provider plugin. `--no-plugin-dirs`
/// is still used for every yt-dlp call, so no arbitrary user plugin is loaded.
pub fn configure_pot_provider(command: &mut std::process::Command, provider: &PotProviderPaths) {
    command
        .arg("--plugin-dirs")
        .arg(&provider.plugin_directory)
        .arg("--extractor-args")
        .arg("youtube:player_client=mweb")
        .arg("--extractor-args")
        .arg(format!("youtubepot-bgutilscript:server_home={}", provider.server_home.display()));
    provider_environment(command, provider);
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
