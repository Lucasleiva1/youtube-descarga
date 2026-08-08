mod download;
mod engine;
mod history;

use serde::Serialize;
use serde_json::Value;
use std::{collections::HashSet, ffi::OsStr, path::PathBuf};
use std::process::Command;
use tauri::Manager;
use url::Url;

const YOUTUBE_BROWSER_RECOVERY_INSTRUCTIONS: &str = "No se pudo validar este video con el acceso público. En Configuración > Acceso a YouTube, elegí Chrome o Edge donde ya usás YouTube; después volvé a Descargas y analizá el enlace nuevamente.";

/// Starts sidecar executables without allowing Windows to create a visible
/// console window for them. yt-dlp, FFmpeg and Deno are console programs, but
/// their output is captured and shown in the application's own interface.
pub(crate) fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineInfo {
    name: String,
    state: String,
    version: Option<String>,
    detail: Option<String>,
    path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VideoFormat {
    id: String,
    extension: String,
    format_note: Option<String>,
    height: Option<u32>,
    width: Option<u32>,
    fps: Option<f64>,
    video_codec: Option<String>,
    audio_codec: Option<String>,
    bitrate: Option<f64>,
    filesize: Option<u64>,
    filesize_approx: Option<u64>,
    has_video: bool,
    has_audio: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QualityOption {
    height: u32,
    label: String,
    format_id: String,
    format_has_audio: bool,
    video_formats: Vec<VideoFormat>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzedVideo {
    id: String,
    url: String,
    title: String,
    channel: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
    browser_session: Option<String>,
    use_pot_provider: bool,
    qualities: Vec<QualityOption>,
    formats: Vec<VideoFormat>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisFailure {
    url: String,
    message: String,
    requires_browser_session: bool,
}

#[derive(Debug, Clone, Serialize)]
struct AnalysisResult {
    videos: Vec<AnalyzedVideo>,
    failures: Vec<AnalysisFailure>,
}

fn probe_command(name: &str, path: Option<PathBuf>, args: &[&str]) -> EngineInfo {
    let Some(program) = path else {
        return EngineInfo {
            name: name.to_owned(), state: "unavailable".to_owned(), version: None,
            detail: Some("No se encontró el sidecar incluido en la aplicación.".to_owned()), path: None,
        };
    };
    let display_path = program.display().to_string();
    match hidden_command(&program).args(args).output() {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => EngineInfo {
            name: name.to_owned(),
            state: "available".to_owned(),
            version: Some(String::from_utf8_lossy(&output.stdout).trim().lines().next().unwrap_or("Disponible").to_owned()),
            detail: None,
            path: Some(display_path),
        },
        Ok(output) => EngineInfo {
            name: name.to_owned(),
            state: "unavailable".to_owned(),
            version: None,
            detail: Some(String::from_utf8_lossy(&output.stderr).trim().lines().next().unwrap_or("El motor no respondió correctamente.").to_owned()),
            path: Some(display_path),
        },
        Err(_) => EngineInfo {
            name: name.to_owned(),
            state: "unavailable".to_owned(),
            version: None,
            detail: Some("No se pudo iniciar el sidecar local.".to_owned()),
            path: Some(display_path),
        },
    }
}

#[tauri::command]
fn check_engines(app: tauri::AppHandle) -> Vec<EngineInfo> {
    let paths = engine::EnginePaths::resolve(&app);
    vec![
        probe_command("yt-dlp", paths.yt_dlp, &["--version"]),
        probe_command("ffmpeg", paths.ffmpeg, &["-version"]),
        probe_command("ffprobe", paths.ffprobe, &["-version"]),
        probe_command("deno", paths.deno, &["--version"]),
    ]
}

fn is_supported_youtube_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value.trim()) else { return false; };
    if !matches!(url.scheme(), "https" | "http") { return false; }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else { return false; };
    host == "youtu.be" || host == "www.youtu.be" || host == "youtube.com" || host.ends_with(".youtube.com")
}

fn browser_session_name(value: Option<&str>) -> Result<Option<&'static str>, String> {
    match value {
        None => Ok(None),
        Some("chrome") => Ok(Some("chrome")),
        Some("edge") => Ok(Some("edge")),
        _ => Err("El navegador seleccionado no es compatible.".to_owned()),
    }
}

/// In public mode, defer to the current yt-dlp defaults instead of pinning a
/// single YouTube client. YouTube changes client availability frequently and
/// yt-dlp can choose the current supported public clients. When the user
/// explicitly opts in to a local browser session, yt-dlp reads it directly
/// from that browser; the application never stores or exports cookies.
pub(crate) fn configure_youtube_access(command: &mut Command, browser_session: Option<&str>) {
    if let Some(browser) = browser_session {
        command.arg("--cookies-from-browser").arg(browser);
    }
}

fn browser_label(browser_session: Option<&str>) -> &'static str {
    match browser_session {
        Some("chrome") => "Chrome",
        Some("edge") => "Edge",
        _ => "el navegador elegido",
    }
}

fn alternate_browser_hint(browser_session: Option<&str>) -> &'static str {
    match browser_session {
        Some("chrome") => "Probá con Edge, cerrándolo por completo antes de reintentar.",
        Some("edge") => "Probá con Chrome si es el navegador donde tenés iniciada tu sesión de YouTube.",
        _ => "Elegí el navegador donde tenés iniciada tu sesión de YouTube.",
    }
}

pub(crate) fn youtube_requires_browser_session(technical: &str) -> bool {
    technical.to_ascii_lowercase().contains("sign in to confirm")
}

/// Converts known yt-dlp/YouTube failures into short, safe messages for the
/// UI. In particular, do not surface browser profile paths, cookie details or
/// command diagnostics. The app deliberately does not attempt to bypass
/// Windows/Chromium credential protection.
pub(crate) fn youtube_failure_message(technical: &str, browser_session: Option<&str>) -> Option<String> {
    let lower = technical.to_ascii_lowercase();
    let browser = browser_label(browser_session);

    if lower.contains("could not copy chrome cookie database")
        || (lower.contains("permission denied") && lower.contains("cookie")) {
        return Some(format!(
            "No se pudo leer la sesión de {browser} porque el navegador mantiene sus datos en uso. Cerralo por completo y reintentá. La aplicación no guardó ni exportó cookies."
        ));
    }
    if lower.contains("failed to decrypt with dpapi")
        || lower.contains("could not decrypt cookies")
        || lower.contains("could not decrypt with dpapi") {
        return Some(format!(
            "{browser} protegió esa sesión con el cifrado de Windows y la aplicación no puede leerla. Para cuidar tu cuenta, no intenta evitar esa protección ni guarda cookies. {}",
            alternate_browser_hint(browser_session)
        ));
    }
    if lower.contains("could not find") && lower.contains("cookies database") {
        return Some(format!(
            "No se encontró un perfil utilizable de {browser}. {}",
            alternate_browser_hint(browser_session)
        ));
    }
    if youtube_requires_browser_session(technical) {
        return Some(match browser_session {
            Some(_) => format!(
                "YouTube no aceptó la sesión de {browser} para este video. Confirmá que estás conectado a YouTube en ese navegador y reintentá."
            ),
            None => "YouTube necesita una sesión local para verificar este video.".to_owned(),
        });
    }
    if lower.contains("private video") {
        return Some("El video es privado.".to_owned());
    }
    if lower.contains("video unavailable") || lower.contains("this video is not available") {
        return Some("El video no está disponible.".to_owned());
    }
    if lower.contains("requested format is not available") {
        return Some("La fuente ya no ofrece el formato original elegido. Volvé a analizar el enlace; la aplicación no descargó una resolución inferior.".to_owned());
    }
    None
}

fn string_field(object: &Value, name: &str) -> Option<String> {
    object.get(name).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn number_field(object: &Value, name: &str) -> Option<f64> {
    object.get(name).and_then(Value::as_f64)
}

fn safe_http_url(value: Option<String>) -> Option<String> {
    value.filter(|candidate| Url::parse(candidate)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "https" | "http")))
}

fn to_video_format(value: &Value) -> Option<VideoFormat> {
    let id = string_field(value, "format_id")?;
    let video_codec = string_field(value, "vcodec");
    let audio_codec = string_field(value, "acodec");
    let has_video = video_codec.as_deref().is_some_and(|codec| codec != "none");
    let has_audio = audio_codec.as_deref().is_some_and(|codec| codec != "none");
    Some(VideoFormat {
        id,
        extension: string_field(value, "ext").unwrap_or_else(|| "unknown".to_owned()),
        format_note: string_field(value, "format_note"),
        height: number_field(value, "height").map(|height| height as u32),
        width: number_field(value, "width").map(|width| width as u32),
        fps: number_field(value, "fps"),
        video_codec,
        audio_codec,
        bitrate: number_field(value, "tbr"),
        filesize: number_field(value, "filesize").map(|size| size as u64),
        filesize_approx: number_field(value, "filesize_approx").map(|size| size as u64),
        has_video,
        has_audio,
    })
}

/// Only direct, usable video streams belong in the resolution picker. This
/// rejects thumbnails/storyboards, DRM entries and incomplete records.
fn is_downloadable_video_format(value: &Value, format: &VideoFormat) -> bool {
    let has_direct_url = value.get("url").and_then(Value::as_str).is_some_and(|url| !url.trim().is_empty());
    let is_drm_protected = value.get("has_drm").and_then(Value::as_bool).unwrap_or(false);
    let is_storyboard = value.get("format_note").and_then(Value::as_str).is_some_and(|note| note.to_ascii_lowercase().contains("storyboard"));
    format.has_video
        && format.width.is_some_and(|width| width > 0)
        && format.height.is_some_and(|height| height > 0)
        && has_direct_url
        && !is_drm_protected
        && !is_storyboard
}

fn preferred_format(left: &VideoFormat, right: &VideoFormat) -> std::cmp::Ordering {
    left.fps.partial_cmp(&right.fps).unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| left.bitrate.partial_cmp(&right.bitrate).unwrap_or(std::cmp::Ordering::Equal))
        .then_with(|| left.filesize.or(left.filesize_approx).cmp(&right.filesize.or(right.filesize_approx)))
        .then_with(|| left.has_audio.cmp(&right.has_audio))
        .then_with(|| left.id.cmp(&right.id))
}

fn quality_label(height: u32) -> String {
    match height {
        4320 => format!("{height}p · 8K"),
        2160 => format!("{height}p · 4K"),
        1440 => format!("{height}p · 2K"),
        1080 => format!("{height}p · Full HD"),
        720 => format!("{height}p · HD"),
        _ => format!("{height}p"),
    }
}

/// YouTube sometimes assigns a standard quality name to a non-standard frame
/// size (for example, a 2:1 movie can be 1280x640 but be advertised as 720p).
/// Prefer that source-provided name instead of fabricating a label from height.
fn source_quality_label(format: &VideoFormat, height: u32) -> String {
    let source_note = format.format_note.as_deref().map(str::trim).filter(|note| {
        note.strip_suffix('p').is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
    });
    match source_note {
        Some(note) if note != format!("{height}p") => format!("{note} ({})", format.resolution_label()),
        Some(note) => match height {
            4320 => format!("{note} · 8K"),
            2160 => format!("{note} · 4K"),
            1440 => format!("{note} · 2K"),
            1080 => format!("{note} · Full HD"),
            720 => format!("{note} · HD"),
            _ => note.to_owned(),
        },
        None => quality_label(height),
    }
}

impl VideoFormat {
    fn resolution_label(&self) -> String {
        match (self.width, self.height) {
            (Some(width), Some(height)) => format!("{width} × {height} px"),
            _ => "tamaño informado por la fuente".to_owned(),
        }
    }
}

fn parse_video(url: String, stdout: &[u8]) -> Result<AnalyzedVideo, String> {
    let data: Value = serde_json::from_slice(stdout).map_err(|_| "yt-dlp devolvió metadata inválida.".to_owned())?;
    let mut formats: Vec<VideoFormat> = data.get("formats").and_then(Value::as_array).into_iter().flatten().filter_map(to_video_format).collect();
    formats.sort_by_key(|format| (format.height.unwrap_or(0), format.fps.unwrap_or(0.0) as u32));
    let downloadable_formats: Vec<VideoFormat> = data.get("formats").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|value| to_video_format(value).filter(|format| is_downloadable_video_format(value, format)))
        .collect();
    let mut heights: Vec<u32> = downloadable_formats.iter().filter_map(|format| format.height).collect();
    heights.sort_unstable();
    heights.dedup();
    let mut qualities: Vec<QualityOption> = heights.into_iter().filter_map(|height| {
        let video_formats: Vec<VideoFormat> = downloadable_formats.iter().filter(|format| format.height == Some(height)).cloned().collect();
        let selected = video_formats.iter().max_by(|left, right| preferred_format(left, right))?;
        Some(QualityOption {
            height,
            label: source_quality_label(selected, height),
            format_id: selected.id.clone(),
            format_has_audio: selected.has_audio,
            video_formats,
        })
    }).collect();
    qualities.sort_by(|left, right| right.height.cmp(&left.height));
    Ok(AnalyzedVideo {
        id: string_field(&data, "id").ok_or_else(|| "No se pudo determinar el ID del video.".to_owned())?,
        url,
        title: string_field(&data, "title").unwrap_or_else(|| "Video sin título".to_owned()),
        channel: string_field(&data, "channel").or_else(|| string_field(&data, "uploader")),
        duration: number_field(&data, "duration"),
        thumbnail: safe_http_url(string_field(&data, "thumbnail")),
        browser_session: None,
        use_pot_provider: false,
        qualities,
        formats,
    })
}

fn analysis_command(
    binary: &std::path::Path,
    ffmpeg_directory: &std::path::Path,
    deno: &std::path::Path,
    browser_session: Option<&str>,
    provider: Option<&engine::PotProviderPaths>,
) -> Command {
    let mut command = hidden_command(binary);
    command.args(["--ignore-config", "--no-plugin-dirs", "--dump-single-json", "--skip-download", "--no-warnings", "--no-playlist", "--ffmpeg-location"])
        .arg(ffmpeg_directory)
        .arg("--js-runtimes")
        .arg(format!("deno:{}", deno.display()));
    configure_youtube_access(&mut command, browser_session);
    if let Some(provider) = provider {
        engine::configure_pot_provider(&mut command, provider);
    }
    command
}

#[tauri::command]
fn analyze_urls(app: tauri::AppHandle, urls: Vec<String>, browser_session: Option<String>) -> Result<AnalysisResult, String> {
    let paths = engine::EnginePaths::resolve(&app);
    paths.required_for_youtube()?;
    let binary = paths.yt_dlp.as_ref().ok_or_else(|| "yt-dlp no está disponible.".to_owned())?;
    let ffmpeg_directory = engine::yt_dlp_ffmpeg_location(&app, &paths)?;
    let deno = paths.deno.as_ref().ok_or_else(|| "No se pudo resolver Deno.".to_owned())?;
    let browser_session = browser_session_name(browser_session.as_deref())?.map(ToOwned::to_owned);
    let mut videos = Vec::new();
    let mut failures = Vec::new();
    let mut seen_urls = HashSet::new();
    let mut seen_video_ids = HashSet::new();
    for url in urls {
        let clean_url = url.trim().to_owned();
        if clean_url.is_empty() { continue; }
        if !seen_urls.insert(clean_url.clone()) {
            failures.push(AnalysisFailure { url: clean_url, message: "URL duplicada: se omitió el análisis repetido.".to_owned(), requires_browser_session: false });
            continue;
        }
        if !is_supported_youtube_url(&clean_url) {
            failures.push(AnalysisFailure { url: clean_url, message: "URL inválida o no compatible. Usá un enlace de YouTube.".to_owned(), requires_browser_session: false });
            continue;
        }
        let mut command = analysis_command(binary, &ffmpeg_directory, deno, browser_session.as_deref(), None);
        let mut output = command.arg(&clean_url)
            .output()
            .map_err(|_| "yt-dlp no está disponible. Instalalo o agregalo a los binarios de la aplicación.".to_owned())?;
        let mut used_pot_provider = false;
        let mut pot_attempted = false;
        if !output.status.success()
            && browser_session.is_none()
            && youtube_requires_browser_session(&String::from_utf8_lossy(&output.stderr))
        {
            pot_attempted = true;
            match engine::ensure_pot_provider(&app, &paths) {
                Ok(provider) => {
                    let mut retry = analysis_command(binary, &ffmpeg_directory, deno, None, Some(&provider));
                    output = retry.arg(&clean_url)
                        .output()
                        .map_err(|_| "yt-dlp no está disponible. Instalalo o agregalo a los binarios de la aplicación.".to_owned())?;
                    used_pot_provider = output.status.success();
                }
                Err(message) => {
                    failures.push(AnalysisFailure { url: clean_url, message, requires_browser_session: false });
                    continue;
                }
            }
        }
        if !output.status.success() {
            let technical = String::from_utf8_lossy(&output.stderr);
            let requires_browser_session = youtube_requires_browser_session(&technical);
            let message = if pot_attempted && requires_browser_session {
                YOUTUBE_BROWSER_RECOVERY_INSTRUCTIONS.to_owned()
            } else if pot_attempted {
                "El verificador local de YouTube no pudo validar este enlace. Comprobá tu conexión y reintentá.".to_owned()
            } else {
                youtube_failure_message(&technical, browser_session.as_deref())
                    .unwrap_or_else(|| "No se pudo obtener la información del video.".to_owned())
            };
            failures.push(AnalysisFailure { url: clean_url, message, requires_browser_session });
            continue;
        }
        match parse_video(clean_url.clone(), &output.stdout) {
            Ok(mut video) if seen_video_ids.insert(video.id.clone()) => {
                video.browser_session = browser_session.clone();
                video.use_pot_provider = used_pot_provider;
                videos.push(video);
            },
            Ok(_) => failures.push(AnalysisFailure { url: clean_url, message: "El video ya fue analizado mediante otra URL.".to_owned(), requires_browser_session: false }),
            Err(message) => failures.push(AnalysisFailure { url: clean_url, message, requires_browser_session: false }),
        }
    }
    Ok(AnalysisResult { videos, failures })
}

#[tauri::command]
fn default_download_directory() -> String {
    std::env::var("USERPROFILE").map(|home| format!("{home}\\Downloads")).unwrap_or_else(|_| "Elegí una carpeta de destino".to_owned())
}

#[tauri::command]
fn add_download_job(app: tauri::AppHandle, request: download::DownloadRequest) -> Result<download::DownloadJob, String> { download::add(app, request) }
#[tauri::command]
fn get_download_queue(app: tauri::AppHandle) -> download::QueueSnapshot { download::get_queue(app) }
#[tauri::command]
fn start_download_queue(app: tauri::AppHandle) -> Result<(), String> { download::start(app) }
#[tauri::command]
fn start_download_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> { download::start_one(app, job_id) }
#[tauri::command]
fn pause_download_queue(app: tauri::AppHandle) { download::pause(app) }
#[tauri::command]
fn resume_download_queue(app: tauri::AppHandle) { download::resume(app) }
#[tauri::command]
fn cancel_download_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> { download::cancel(app, job_id) }
#[tauri::command]
fn cancel_all_downloads(app: tauri::AppHandle) -> Result<(), String> { download::cancel_all(app) }
#[tauri::command]
fn clear_finished_downloads(app: tauri::AppHandle) -> Result<(), String> { download::clear_finished(app) }
#[tauri::command]
fn retry_download_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> { download::retry(app, job_id) }
#[tauri::command]
fn open_download_file(app: tauri::AppHandle, job_id: String) -> Result<(), String> { download::open_file(app, job_id) }
#[tauri::command]
fn open_download_folder(app: tauri::AppHandle, job_id: String) -> Result<(), String> { download::open_folder(app, job_id) }
#[tauri::command]
fn get_history(app: tauri::AppHandle) -> Result<Vec<history::HistoryEntry>, String> { history::list(&app) }
#[tauri::command]
fn remove_history_entry(app: tauri::AppHandle, id: String) -> Result<(), String> { history::remove(&app, &id) }

#[cfg(test)]
mod tests {
    use super::{parse_video, youtube_failure_message, youtube_requires_browser_session, YOUTUBE_BROWSER_RECOVERY_INSTRUCTIONS};
    use serde_json::json;

    #[test]
    fn exposes_only_direct_downloadable_resolutions() {
        let metadata = json!({
            "id": "example",
            "title": "Example",
            "formats": [
                { "format_id": "401", "ext": "webm", "width": 3840, "height": 2160, "fps": 30, "vcodec": "av01", "acodec": "none", "tbr": 12000, "format_note": "2160p", "url": "https://example.test/2160" },
                { "format_id": "399", "ext": "mp4", "width": 1920, "height": 1080, "fps": 60, "vcodec": "avc1", "acodec": "none", "tbr": 7000, "url": "https://example.test/1080" },
                { "format_id": "sb0", "ext": "mhtml", "width": 1920, "height": 1080, "vcodec": "avc1", "acodec": "none", "format_note": "storyboard", "url": "https://example.test/storyboard" },
                { "format_id": "drm", "ext": "mp4", "width": 1280, "height": 720, "vcodec": "avc1", "acodec": "none", "has_drm": true, "url": "https://example.test/drm" },
                { "format_id": "missing", "ext": "mp4", "width": 854, "height": 480, "vcodec": "avc1", "acodec": "none" }
            ]
        });
        let video = parse_video("https://www.youtube.com/watch?v=example".to_owned(), metadata.to_string().as_bytes()).expect("metadata should parse");
        let heights: Vec<u32> = video.qualities.iter().map(|quality| quality.height).collect();
        assert_eq!(heights, vec![2160, 1080]);
        assert_eq!(video.qualities[0].format_id, "401");
    }

    #[test]
    fn keeps_the_source_quality_name_for_nonstandard_frame_sizes() {
        let metadata = json!({
            "id": "cinema", "title": "Cinema",
            "formats": [{ "format_id": "398", "ext": "mp4", "width": 1280, "height": 640, "vcodec": "avc1", "acodec": "none", "format_note": "720p", "url": "https://example.test/720" }]
        });
        let video = parse_video("https://www.youtube.com/watch?v=cinema".to_owned(), metadata.to_string().as_bytes()).expect("metadata should parse");
        assert_eq!(video.qualities[0].label, "720p (1280 × 640 px)");
    }

    #[test]
    fn explains_when_a_chromium_cookie_database_is_locked_without_exposing_it() {
        let message = youtube_failure_message(
            "ERROR: Could not copy Chrome cookie database. Permission denied: C:\\Users\\person\\AppData\\Local\\Microsoft\\Edge\\User Data\\Default\\Network\\Cookies",
            Some("edge"),
        ).expect("known browser lock should have a safe message");
        assert!(message.contains("Edge"));
        assert!(message.contains("Cerralo por completo"));
        assert!(!message.contains("C:\\Users"));
        assert!(!message.contains("Network\\Cookies"));
    }

    #[test]
    fn explains_dpapi_protection_without_claiming_to_bypass_it() {
        let message = youtube_failure_message("ERROR: Failed to decrypt with DPAPI", Some("chrome"))
            .expect("DPAPI failure should have a safe message");
        assert!(message.contains("Chrome"));
        assert!(message.contains("no intenta evitar esa protección"));
    }

    #[test]
    fn keeps_the_browser_recovery_flag_for_youtube_antibot_challenges() {
        assert!(youtube_requires_browser_session("ERROR: Sign in to confirm you’re not a bot"));
        assert!(!youtube_requires_browser_session("ERROR: Could not copy Chrome cookie database"));
    }

    #[test]
    fn explains_the_manual_browser_recovery_path_after_local_verification_fails() {
        assert!(YOUTUBE_BROWSER_RECOVERY_INSTRUCTIONS.contains("Configuración > Acceso a YouTube"));
        assert!(YOUTUBE_BROWSER_RECOVERY_INSTRUCTIONS.contains("Chrome o Edge"));
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(download::DownloadManager::default())
        .setup(|app| {
            history::initialize(&app.handle()).map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            // A process launched by a development host can inherit a minimized
            // show-state on Windows. Always present the main desktop window.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![check_engines, analyze_urls, default_download_directory, add_download_job, get_download_queue, start_download_queue, start_download_job, pause_download_queue, resume_download_queue, cancel_download_job, cancel_all_downloads, clear_finished_downloads, retry_download_job, open_download_file, open_download_folder, get_history, remove_history_entry])
        .build(tauri::generate_context!())
        .expect("error while building YT Download");

    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Ready) {
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    });
}
