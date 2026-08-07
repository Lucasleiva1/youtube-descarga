mod engine;

use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

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
    qualities: Vec<QualityOption>,
    formats: Vec<VideoFormat>,
}

#[derive(Debug, Clone, Serialize)]
struct AnalysisFailure {
    url: String,
    message: String,
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
    match Command::new(&program).args(args).output() {
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
    let lower = value.trim().to_ascii_lowercase();
    let has_host = lower.contains("youtube.com/") || lower.contains("youtu.be/");
    has_host && (lower.starts_with("https://") || lower.starts_with("http://"))
}

fn string_field(object: &Value, name: &str) -> Option<String> {
    object.get(name).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn number_field(object: &Value, name: &str) -> Option<f64> {
    object.get(name).and_then(Value::as_f64)
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

fn quality_label(height: u32) -> String {
    match height {
        2160.. => format!("{height}p · 4K"),
        1440.. => format!("{height}p · 2K"),
        1080.. => format!("{height}p · Full HD"),
        720.. => format!("{height}p · HD"),
        _ => format!("{height}p"),
    }
}

fn parse_video(url: String, stdout: &[u8]) -> Result<AnalyzedVideo, String> {
    let data: Value = serde_json::from_slice(stdout).map_err(|_| "yt-dlp devolvió metadata inválida.".to_owned())?;
    let mut formats: Vec<VideoFormat> = data.get("formats").and_then(Value::as_array).into_iter().flatten().filter_map(to_video_format).collect();
    formats.sort_by_key(|format| (format.height.unwrap_or(0), format.fps.unwrap_or(0.0) as u32));
    let mut heights: Vec<u32> = formats.iter().filter(|format| format.has_video).filter_map(|format| format.height).collect();
    heights.sort_unstable();
    heights.dedup();
    let qualities = heights.into_iter().map(|height| QualityOption {
        height,
        label: quality_label(height),
        video_formats: formats.iter().filter(|format| format.has_video && format.height == Some(height)).cloned().collect(),
    }).collect();
    Ok(AnalyzedVideo {
        id: string_field(&data, "id").ok_or_else(|| "No se pudo determinar el ID del video.".to_owned())?,
        url,
        title: string_field(&data, "title").unwrap_or_else(|| "Video sin título".to_owned()),
        channel: string_field(&data, "channel").or_else(|| string_field(&data, "uploader")),
        duration: number_field(&data, "duration"),
        thumbnail: string_field(&data, "thumbnail"),
        qualities,
        formats,
    })
}

#[tauri::command]
fn analyze_urls(app: tauri::AppHandle, urls: Vec<String>) -> Result<AnalysisResult, String> {
    let paths = engine::EnginePaths::resolve(&app);
    paths.required_for_youtube()?;
    let binary = paths.yt_dlp.ok_or_else(|| "yt-dlp no está disponible.".to_owned())?;
    let ffmpeg_directory = paths.ffmpeg.as_ref().and_then(|path| path.parent()).ok_or_else(|| "No se pudo resolver FFmpeg.".to_owned())?.to_path_buf();
    let deno = paths.deno.ok_or_else(|| "No se pudo resolver Deno.".to_owned())?;
    let mut videos = Vec::new();
    let mut failures = Vec::new();
    for url in urls {
        let clean_url = url.trim().to_owned();
        if !is_supported_youtube_url(&clean_url) {
            failures.push(AnalysisFailure { url: clean_url, message: "URL inválida o no compatible. Usá un enlace de YouTube.".to_owned() });
            continue;
        }
        let output = Command::new(&binary)
            .args(["--dump-single-json", "--skip-download", "--no-warnings", "--no-playlist", "--ffmpeg-location"])
            .arg(&ffmpeg_directory)
            .arg("--js-runtimes")
            .arg(format!("deno:{}", deno.display()))
            .arg(&clean_url)
            .output()
            .map_err(|_| "yt-dlp no está disponible. Instalalo o agregalo a los binarios de la aplicación.".to_owned())?;
        if !output.status.success() {
            let technical = String::from_utf8_lossy(&output.stderr);
            let message = if technical.contains("Private video") { "El video es privado." } else if technical.contains("Video unavailable") { "El video no está disponible." } else { "No se pudo obtener la información del video." };
            failures.push(AnalysisFailure { url: clean_url, message: message.to_owned() });
            continue;
        }
        match parse_video(clean_url.clone(), &output.stdout) {
            Ok(video) => videos.push(video),
            Err(message) => failures.push(AnalysisFailure { url: clean_url, message }),
        }
    }
    Ok(AnalysisResult { videos, failures })
}

#[tauri::command]
fn default_download_directory() -> String {
    std::env::var("USERPROFILE").map(|home| format!("{home}\\Downloads")).unwrap_or_else(|_| "Elegí una carpeta de destino".to_owned())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![check_engines, analyze_urls, default_download_directory])
        .run(tauri::generate_context!())
        .expect("error while running YT Download");
}
