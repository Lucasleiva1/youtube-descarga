mod download;
mod engine;
mod history;
mod youtube_access;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    ffi::OsStr,
    io::Read,
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use tauri::Manager;
use url::Url;

const ANALYSIS_TIMEOUT: Duration = Duration::from_secs(90);

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QualityOption {
    height: u32,
    label: String,
    format_id: String,
    format_has_audio: bool,
    video_formats: Vec<VideoFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzedVideo {
    id: String,
    url: String,
    title: String,
    channel: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
    access_strategy: YoutubeAccessStrategy,
    // Legacy fields remain serialized for compatibility with already-built
    // frontends. New clients should round-trip `accessStrategy` instead.
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
    existing_download_id: Option<String>,
    retry_after_epoch: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct AnalysisResult {
    videos: Vec<AnalyzedVideo>,
    failures: Vec<AnalysisFailure>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum BrowserSession {
    Firefox,
    Edge,
    Chrome,
    Brave,
}

impl BrowserSession {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Firefox => "firefox",
            Self::Edge => "edge",
            Self::Chrome => "chrome",
            Self::Brave => "brave",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Firefox => "Firefox",
            Self::Edge => "Edge",
            Self::Chrome => "Chrome",
            Self::Brave => "Brave",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "firefox" => Some(Self::Firefox),
            "edge" => Some(Self::Edge),
            "chrome" => Some(Self::Chrome),
            "brave" => Some(Self::Brave),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum YoutubeAccessStrategy {
    #[default]
    Public,
    Pot,
    Browser {
        browser: BrowserSession,
        #[serde(default, rename = "usePotProvider")]
        use_pot_provider: bool,
    },
}

impl YoutubeAccessStrategy {
    pub(crate) fn browser(&self) -> Option<BrowserSession> {
        match self {
            Self::Browser { browser, .. } => Some(*browser),
            _ => None,
        }
    }

    pub(crate) fn uses_pot_provider(&self) -> bool {
        matches!(
            self,
            Self::Pot
                | Self::Browser {
                    use_pot_provider: true,
                    ..
                }
        )
    }

    fn diagnostic_name(&self) -> String {
        match self {
            Self::Public => "public".to_owned(),
            Self::Pot => "pot_http".to_owned(),
            Self::Browser {
                browser,
                use_pot_provider: false,
            } => format!("browser_{}", browser.as_str()),
            Self::Browser {
                browser,
                use_pot_provider: true,
            } => format!("browser_{}_pot_http", browser.as_str()),
        }
    }

    fn legacy_fields(&self) -> (Option<String>, bool) {
        (
            self.browser().map(|browser| browser.as_str().to_owned()),
            self.uses_pot_provider(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum YoutubeFailureClass {
    AntiBotChallenge,
    AccountRequired,
    PotRequired,
    PotUnavailable,
    CookieDatabaseLocked,
    CookieDecryptUnsupported,
    BrowserMissing,
    SessionRejected,
    ExtractorOutdated,
    RequestedFormatUnavailable,
    Private,
    Unavailable,
    GeoRestricted,
    RateLimited,
    Network,
    TimedOut,
    Other,
}

impl YoutubeFailureClass {
    fn code(self) -> &'static str {
        match self {
            Self::AntiBotChallenge => "anti_bot",
            Self::AccountRequired => "account_required",
            Self::PotRequired => "pot_required",
            Self::PotUnavailable => "pot_unavailable",
            Self::CookieDatabaseLocked => "cookie_db_locked",
            Self::CookieDecryptUnsupported => "cookie_decrypt_unsupported",
            Self::BrowserMissing => "browser_missing",
            Self::SessionRejected => "session_rejected",
            Self::ExtractorOutdated => "extractor_outdated",
            Self::RequestedFormatUnavailable => "format_unavailable",
            Self::Private => "private",
            Self::Unavailable => "unavailable",
            Self::GeoRestricted => "geo_restricted",
            Self::RateLimited => "rate_limited",
            Self::Network => "network",
            Self::TimedOut => "timeout",
            Self::Other => "other",
        }
    }

    fn warrants_pot(self) -> bool {
        matches!(self, Self::AntiBotChallenge | Self::PotRequired)
    }

    fn is_account_requirement(self) -> bool {
        matches!(self, Self::AccountRequired)
    }
}

fn probe_command(name: &str, path: Option<PathBuf>, args: &[&str]) -> EngineInfo {
    let Some(program) = path else {
        return EngineInfo {
            name: name.to_owned(),
            state: "unavailable".to_owned(),
            version: None,
            detail: Some("No se encontró el sidecar incluido en la aplicación.".to_owned()),
            path: None,
        };
    };
    let display_path = program.display().to_string();
    match hidden_command(&program).args(args).output() {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => EngineInfo {
            name: name.to_owned(),
            state: "available".to_owned(),
            version: Some(
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .lines()
                    .next()
                    .unwrap_or("Disponible")
                    .to_owned(),
            ),
            detail: None,
            path: Some(display_path),
        },
        Ok(output) => EngineInfo {
            name: name.to_owned(),
            state: "unavailable".to_owned(),
            version: None,
            detail: Some(
                String::from_utf8_lossy(&output.stderr)
                    .trim()
                    .lines()
                    .next()
                    .unwrap_or("El motor no respondió correctamente.")
                    .to_owned(),
            ),
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
    let Ok(url) = Url::parse(value.trim()) else {
        return false;
    };
    if !matches!(url.scheme(), "https" | "http") {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    host == "youtu.be"
        || host == "www.youtu.be"
        || host == "youtube.com"
        || host.ends_with(".youtube.com")
}

fn youtube_video_id(value: &str) -> Option<String> {
    let url = Url::parse(value.trim()).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let candidate = if matches!(host.as_str(), "youtu.be" | "www.youtu.be") {
        url.path_segments()?
            .find(|segment| !segment.is_empty())?
            .to_owned()
    } else if host == "youtube.com" || host.ends_with(".youtube.com") {
        let mut segments = url.path_segments()?;
        match segments.next().filter(|segment| !segment.is_empty()) {
            Some("watch") | None => url
                .query_pairs()
                .find_map(|(key, value)| (key == "v").then(|| value.into_owned()))?,
            Some("shorts" | "embed" | "live" | "v") => segments.next()?.to_owned(),
            _ => return None,
        }
    } else {
        return None;
    };
    let candidate = candidate.trim();
    (6..=32)
        .contains(&candidate.len())
        .then(|| candidate.to_owned())
        .filter(|id| {
            id.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
}

/// Applies only the already-resolved access strategy. Strategy discovery lives
/// in `analyze_url_automatically`, so public access is always attempted first.
pub(crate) fn configure_youtube_access(command: &mut Command, strategy: &YoutubeAccessStrategy) {
    if let Some(browser) = strategy.browser() {
        command.arg("--cookies-from-browser").arg(browser.as_str());
    }
}

/// Converts unstable yt-dlp text into a small internal taxonomy. Raw stderr is
/// never returned to the frontend or written to the diagnostic log.
pub(crate) fn classify_youtube_failure(technical: &str) -> YoutubeFailureClass {
    let lower = technical.to_ascii_lowercase();
    if lower.contains("could not copy chrome cookie database")
        || (lower.contains("permission denied") && lower.contains("cookie"))
    {
        return YoutubeFailureClass::CookieDatabaseLocked;
    }
    if lower.contains("failed to decrypt with dpapi")
        || lower.contains("could not decrypt cookies")
        || lower.contains("could not decrypt with dpapi")
        || lower.contains("cannot decrypt v20")
        || lower.contains("unknown cookie version")
    {
        return YoutubeFailureClass::CookieDecryptUnsupported;
    }
    if (lower.contains("could not find") && lower.contains("cookies database"))
        || lower.contains("unsupported browser")
    {
        return YoutubeFailureClass::BrowserMissing;
    }
    if lower.contains("private video") {
        return if lower.contains("sign in") || lower.contains("granted access") {
            YoutubeFailureClass::AccountRequired
        } else {
            YoutubeFailureClass::Private
        };
    }
    if lower.contains("not available in your country")
        || lower.contains("not available in your region")
        || lower.contains("geo-restricted")
    {
        return YoutubeFailureClass::GeoRestricted;
    }
    if lower.contains("video unavailable") || lower.contains("this video is not available") {
        return YoutubeFailureClass::Unavailable;
    }
    if lower.contains("requested format is not available") {
        return YoutubeFailureClass::RequestedFormatUnavailable;
    }
    if lower.contains("http error 429") || lower.contains("too many requests") {
        return YoutubeFailureClass::RateLimited;
    }
    if lower.contains("sign in to confirm")
        || lower.contains("confirm you’re not a bot")
        || lower.contains("confirm you're not a bot")
        || lower.contains("confirm you are not a bot")
    {
        return YoutubeFailureClass::AntiBotChallenge;
    }
    if lower.contains("members-only")
        || lower.contains("members only")
        || lower.contains("join this channel")
        || lower.contains("login required")
        || lower.contains("log in to view")
        || lower.contains("sign in to view")
        || lower.contains("confirm your age")
        || lower.contains("age-restricted")
    {
        return YoutubeFailureClass::AccountRequired;
    }
    if lower.contains("po token")
        || lower.contains("proof of origin")
        || lower.contains("pot provider")
        || lower.contains("pot is required")
    {
        return YoutubeFailureClass::PotRequired;
    }
    if lower.contains("please update")
        || lower.contains("update to a nightly")
        || lower.contains("signature solving failed")
        || lower.contains("nsig extraction failed")
        || lower.contains("no longer supported") && lower.contains("youtube")
    {
        return YoutubeFailureClass::ExtractorOutdated;
    }
    if lower.contains("http error 401") || lower.contains("http error 403") {
        return YoutubeFailureClass::SessionRejected;
    }
    if lower.contains("timed out")
        || lower.contains("temporary failure in name resolution")
        || lower.contains("unable to download webpage")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("network is unreachable")
    {
        return YoutubeFailureClass::Network;
    }
    YoutubeFailureClass::Other
}

#[cfg(test)]
pub(crate) fn youtube_requires_browser_session(technical: &str) -> bool {
    classify_youtube_failure(technical).is_account_requirement()
}

/// Produces only safe, actionable text. It deliberately omits raw yt-dlp
/// diagnostics because those can contain local profile paths and request data.
pub(crate) fn youtube_failure_message(
    technical: &str,
    strategy: &YoutubeAccessStrategy,
) -> Option<String> {
    let browser = strategy
        .browser()
        .map(BrowserSession::label)
        .unwrap_or("el navegador local");
    Some(match classify_youtube_failure(technical) {
        YoutubeFailureClass::CookieDatabaseLocked => format!(
            "No se pudo usar la sesión de {browser} porque su base de datos está abierta. La aplicación continuó con las demás opciones sin cerrar el navegador."
        ),
        YoutubeFailureClass::CookieDecryptUnsupported => format!(
            "La protección de cookies de {browser} no es compatible con el motor actual. La aplicación no desactivó el cifrado y continuó con las demás opciones."
        ),
        YoutubeFailureClass::BrowserMissing => format!(
            "No se encontró una sesión utilizable de {browser}."
        ),
        YoutubeFailureClass::AntiBotChallenge => {
            "YouTube no aceptó todavía la verificación automática.".to_owned()
        }
        YoutubeFailureClass::AccountRequired => {
            "YouTube exige una cuenta con acceso a este contenido y no se encontró una sesión local válida.".to_owned()
        }
        YoutubeFailureClass::PotRequired | YoutubeFailureClass::PotUnavailable => {
            "El verificador local de YouTube no estuvo disponible.".to_owned()
        }
        YoutubeFailureClass::Private => "El video es privado.".to_owned(),
        YoutubeFailureClass::Unavailable => "El video no está disponible.".to_owned(),
        YoutubeFailureClass::GeoRestricted => {
            "El video no está disponible en esta región.".to_owned()
        }
        YoutubeFailureClass::RateLimited => "YouTube limitó temporalmente las solicitudes. Esperá unos minutos antes de reintentar.".to_owned(),
        YoutubeFailureClass::RequestedFormatUnavailable => "La fuente ya no ofrece el formato original elegido; se volverá a buscar la misma resolución.".to_owned(),
        YoutubeFailureClass::TimedOut => "YouTube tardó demasiado en responder.".to_owned(),
        YoutubeFailureClass::Network => "No se pudo conectar con YouTube. Comprobá la conexión y reintentá.".to_owned(),
        YoutubeFailureClass::ExtractorOutdated => "YouTube cambió su extractor y el motor local necesita actualizarse.".to_owned(),
        YoutubeFailureClass::SessionRejected => "YouTube rechazó esta sesión local.".to_owned(),
        YoutubeFailureClass::Other => return None,
    })
}

fn string_field(object: &Value, name: &str) -> Option<String> {
    object
        .get(name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn number_field(object: &Value, name: &str) -> Option<f64> {
    object.get(name).and_then(Value::as_f64)
}

fn safe_http_url(value: Option<String>) -> Option<String> {
    value.filter(|candidate| {
        Url::parse(candidate)
            .ok()
            .is_some_and(|url| matches!(url.scheme(), "https" | "http"))
    })
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
    let has_direct_url = value
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(|url| !url.trim().is_empty());
    let is_drm_protected = value
        .get("has_drm")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let is_storyboard = value
        .get("format_note")
        .and_then(Value::as_str)
        .is_some_and(|note| note.to_ascii_lowercase().contains("storyboard"));
    format.has_video
        && format.width.is_some_and(|width| width > 0)
        && format.height.is_some_and(|height| height > 0)
        && has_direct_url
        && !is_drm_protected
        && !is_storyboard
}

fn preferred_format(left: &VideoFormat, right: &VideoFormat) -> std::cmp::Ordering {
    left.fps
        .partial_cmp(&right.fps)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            left.bitrate
                .partial_cmp(&right.bitrate)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            left.filesize
                .or(left.filesize_approx)
                .cmp(&right.filesize.or(right.filesize_approx))
        })
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
        note.strip_suffix('p').is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    });
    match source_note {
        Some(note) if note != format!("{height}p") => {
            format!("{note} ({})", format.resolution_label())
        }
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
    let data: Value = serde_json::from_slice(stdout)
        .map_err(|_| "yt-dlp devolvió metadata inválida.".to_owned())?;
    let downloadable_formats: Vec<VideoFormat> = data
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            to_video_format(value).filter(|format| {
                is_downloadable_video_format(value, format) && !format.id.ends_with("-sr")
            })
        })
        .collect();
    let mut heights: Vec<u32> = downloadable_formats
        .iter()
        .filter_map(|format| format.height)
        .collect();
    heights.sort_unstable();
    heights.dedup();
    let mut qualities: Vec<QualityOption> = heights
        .into_iter()
        .filter_map(|height| {
            let video_formats: Vec<VideoFormat> = downloadable_formats
                .iter()
                .filter(|format| format.height == Some(height))
                .cloned()
                .collect();
            let selected = video_formats
                .iter()
                .max_by(|left, right| preferred_format(left, right))?;
            Some(QualityOption {
                height,
                label: source_quality_label(selected, height),
                format_id: selected.id.clone(),
                format_has_audio: selected.has_audio,
                video_formats,
            })
        })
        .collect();
    qualities.sort_by_key(|quality| std::cmp::Reverse(quality.height));
    Ok(AnalyzedVideo {
        id: string_field(&data, "id")
            .ok_or_else(|| "No se pudo determinar el ID del video.".to_owned())?,
        url,
        title: string_field(&data, "title").unwrap_or_else(|| "Video sin título".to_owned()),
        channel: string_field(&data, "channel").or_else(|| string_field(&data, "uploader")),
        duration: number_field(&data, "duration"),
        thumbnail: safe_http_url(string_field(&data, "thumbnail")),
        access_strategy: YoutubeAccessStrategy::Public,
        browser_session: None,
        use_pot_provider: false,
        qualities,
        formats: downloadable_formats,
    })
}

fn analysis_command(
    binary: &std::path::Path,
    ffmpeg_directory: &std::path::Path,
    deno: &std::path::Path,
    strategy: &YoutubeAccessStrategy,
    provider: Option<&engine::PotProviderPaths>,
) -> Command {
    let mut command = hidden_command(binary);
    command
        .args([
            "--ignore-config",
            "--no-plugin-dirs",
            "--dump-single-json",
            "--skip-download",
            "--no-playlist",
            "--force-ipv4",
            "--socket-timeout",
            "20",
            "--ffmpeg-location",
        ])
        .arg(ffmpeg_directory)
        .arg("--js-runtimes")
        .arg(format!("deno:{}", deno.display()));
    youtube_access::configure_extraction_pacing(&mut command);
    configure_youtube_access(&mut command, strategy);
    if let Some(provider) = provider {
        engine::configure_pot_provider(&mut command, provider);
    }
    command
}

fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<Output, YoutubeFailureClass> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|_| YoutubeFailureClass::Other)?;
    let stdout = child.stdout.take().ok_or(YoutubeFailureClass::Other)?;
    let stderr = child.stderr.take().ok_or(YoutubeFailureClass::Other)?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::BufReader::new(stdout).read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::BufReader::new(stderr).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(YoutubeFailureClass::TimedOut);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(YoutubeFailureClass::Other);
            }
        }
    };
    Ok(Output {
        status,
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
    })
}

#[derive(Debug, Clone)]
struct AnalysisAttemptFailure {
    class: YoutubeFailureClass,
    message: Option<String>,
}

fn yt_dlp_version(binary: &std::path::Path) -> String {
    hidden_command(binary)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().chars().take(32).collect())
        .filter(|version: &String| !version.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

#[allow(clippy::too_many_arguments)]
fn run_analysis_attempt(
    app: &tauri::AppHandle,
    paths: &engine::EnginePaths,
    binary: &std::path::Path,
    ffmpeg_directory: &std::path::Path,
    deno: &std::path::Path,
    url: &str,
    strategy: YoutubeAccessStrategy,
    engine_version: &str,
) -> Result<AnalyzedVideo, AnalysisAttemptFailure> {
    let started = Instant::now();
    youtube_access::begin_network_operation(app).map_err(|message| AnalysisAttemptFailure {
        class: YoutubeFailureClass::RateLimited,
        message: Some(message),
    })?;
    let result = (|| {
        let provider = if strategy.uses_pot_provider() {
            Some(engine::ensure_pot_provider(app, paths).map_err(|message| {
                AnalysisAttemptFailure {
                    class: YoutubeFailureClass::PotUnavailable,
                    message: Some(message),
                }
            })?)
        } else {
            None
        };
        let mut command =
            analysis_command(binary, ffmpeg_directory, deno, &strategy, provider.as_ref());
        command.arg(url);
        let output = output_with_timeout(&mut command, ANALYSIS_TIMEOUT).map_err(|class| {
            AnalysisAttemptFailure {
                class,
                message: None,
            }
        })?;
        if !output.status.success() {
            let technical = String::from_utf8_lossy(&output.stderr);
            return Err(AnalysisAttemptFailure {
                class: classify_youtube_failure(&technical),
                message: youtube_failure_message(&technical, &strategy),
            });
        }
        let mut video = parse_video(url.to_owned(), &output.stdout).map_err(|message| {
            AnalysisAttemptFailure {
                class: YoutubeFailureClass::Other,
                message: Some(message),
            }
        })?;
        let (browser_session, use_pot_provider) = strategy.legacy_fields();
        video.access_strategy = strategy.clone();
        video.browser_session = browser_session;
        video.use_pot_provider = use_pot_provider;
        Ok(video)
    })();
    youtube_access::finish_network_operation(app);
    let outcome = match &result {
        Ok(_) => "success",
        Err(failure) => failure.class.code(),
    };
    engine::diagnostic_log(
        app,
        &format!(
            "component=analysis strategy={} duration_ms={} outcome={} engine={engine_version}",
            strategy.diagnostic_name(),
            started.elapsed().as_millis(),
            outcome
        ),
    );
    result
}

fn automatic_failure(url: String, attempts: &[AnalysisAttemptFailure]) -> AnalysisFailure {
    let requires_browser_session = attempts
        .iter()
        .any(|attempt| attempt.class.is_account_requirement());
    let terminal = [
        YoutubeFailureClass::Private,
        YoutubeFailureClass::GeoRestricted,
        YoutubeFailureClass::RateLimited,
        YoutubeFailureClass::Unavailable,
        YoutubeFailureClass::TimedOut,
        YoutubeFailureClass::Network,
        YoutubeFailureClass::ExtractorOutdated,
    ]
    .into_iter()
    .find_map(|class| attempts.iter().find(|attempt| attempt.class == class));
    let message = if requires_browser_session {
        "Este contenido exige una cuenta. Por privacidad, la aplicación no lee ni usa sesiones de tus navegadores y no realizó solicitudes autenticadas.".to_owned()
    } else if let Some(failure) = terminal {
        failure
            .message
            .clone()
            .unwrap_or_else(|| "No se pudo obtener la información del video.".to_owned())
    } else if attempts
        .iter()
        .any(|attempt| attempt.class == YoutubeFailureClass::AntiBotChallenge)
    {
        "No se pudo completar la verificación automática de YouTube mediante el acceso público y el verificador local.".to_owned()
    } else if attempts
        .iter()
        .any(|attempt| attempt.class == YoutubeFailureClass::PotUnavailable)
    {
        "El verificador local de YouTube no estuvo disponible. Comprobá la conexión y reintentá."
            .to_owned()
    } else {
        attempts
            .iter()
            .rev()
            .find_map(|attempt| attempt.message.clone())
            .unwrap_or_else(|| "No se pudo obtener la información del video.".to_owned())
    };
    AnalysisFailure {
        url,
        message,
        requires_browser_session,
        existing_download_id: None,
        retry_after_epoch: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn analyze_url_automatically(
    app: &tauri::AppHandle,
    paths: &engine::EnginePaths,
    binary: &std::path::Path,
    ffmpeg_directory: &std::path::Path,
    deno: &std::path::Path,
    url: &str,
    engine_version: &str,
) -> Result<AnalyzedVideo, AnalysisFailure> {
    let mut attempts = Vec::new();
    match run_analysis_attempt(
        app,
        paths,
        binary,
        ffmpeg_directory,
        deno,
        url,
        YoutubeAccessStrategy::Public,
        engine_version,
    ) {
        Ok(video) => return Ok(video),
        Err(failure) => attempts.push(failure),
    }
    let public_class = attempts[0].class;
    if public_class == YoutubeFailureClass::RateLimited {
        youtube_access::activate_cooldown(app, "rate_limited");
    }
    if matches!(
        public_class,
        YoutubeFailureClass::Private
            | YoutubeFailureClass::Unavailable
            | YoutubeFailureClass::GeoRestricted
            | YoutubeFailureClass::RateLimited
            | YoutubeFailureClass::Network
            | YoutubeFailureClass::TimedOut
            | YoutubeFailureClass::Other
    ) {
        return Err(automatic_failure(url.to_owned(), &attempts));
    }

    if public_class.warrants_pot() {
        match run_analysis_attempt(
            app,
            paths,
            binary,
            ffmpeg_directory,
            deno,
            url,
            YoutubeAccessStrategy::Pot,
            engine_version,
        ) {
            Ok(video) => return Ok(video),
            Err(failure) => attempts.push(failure),
        }
        if let Some(pot_failure) = attempts.last() {
            if pot_failure.class == YoutubeFailureClass::RateLimited
                || (public_class == YoutubeFailureClass::AntiBotChallenge
                    && pot_failure.class == YoutubeFailureClass::AntiBotChallenge)
            {
                youtube_access::activate_cooldown(app, pot_failure.class.code());
            }
        }
    }

    Err(automatic_failure(url.to_owned(), &attempts))
}

fn analyze_urls_blocking(
    app: tauri::AppHandle,
    urls: Vec<String>,
    ignore_history: bool,
) -> Result<AnalysisResult, String> {
    let paths = engine::EnginePaths::resolve(&app);
    paths.required_for_youtube()?;
    let binary = paths
        .yt_dlp
        .as_ref()
        .ok_or_else(|| "yt-dlp no está disponible.".to_owned())?;
    let ffmpeg_directory = engine::yt_dlp_ffmpeg_location(&app, &paths)?;
    let deno = paths
        .deno
        .as_ref()
        .ok_or_else(|| "No se pudo resolver Deno.".to_owned())?;
    let engine_version = yt_dlp_version(binary);
    let mut videos = Vec::new();
    let mut failures = Vec::new();
    let mut seen_video_ids = HashSet::new();
    for url in urls {
        let clean_url = url.trim().to_owned();
        if clean_url.is_empty() {
            continue;
        }
        if !is_supported_youtube_url(&clean_url) {
            failures.push(AnalysisFailure {
                url: clean_url,
                message: "URL inválida o no compatible. Usá un enlace de YouTube.".to_owned(),
                requires_browser_session: false,
                existing_download_id: None,
                retry_after_epoch: None,
            });
            continue;
        }
        let Some(video_id) = youtube_video_id(&clean_url) else {
            failures.push(AnalysisFailure {
                url: clean_url,
                message: "No se pudo obtener localmente el ID del video. Usá un enlace directo de YouTube.".to_owned(),
                requires_browser_session: false,
                existing_download_id: None,
                retry_after_epoch: None,
            });
            continue;
        };
        if !seen_video_ids.insert(video_id.clone()) {
            failures.push(AnalysisFailure {
                url: clean_url,
                message: "El video está repetido en la lista; se omitió sin consultar YouTube."
                    .to_owned(),
                requires_browser_session: false,
                existing_download_id: None,
                retry_after_epoch: None,
            });
            continue;
        }
        if !ignore_history {
            if let Some(existing) = history::find_existing_by_video_id(&app, &video_id)? {
                failures.push(AnalysisFailure {
                    url: clean_url,
                    message: "Este video ya está descargado y el archivo sigue disponible. No se volvió a consultar YouTube.".to_owned(),
                    requires_browser_session: false,
                    existing_download_id: Some(existing.id),
                    retry_after_epoch: None,
                });
                continue;
            }
        }
        if let Some(video) = youtube_access::cached_analysis(&app, &video_id, &clean_url) {
            videos.push(video);
            continue;
        }
        if let Some(until) = youtube_access::cooldown_until(&app) {
            failures.push(AnalysisFailure {
                url: clean_url,
                message: youtube_access::cooldown_message(until),
                requires_browser_session: false,
                existing_download_id: None,
                retry_after_epoch: Some(until),
            });
            continue;
        }
        match analyze_url_automatically(
            &app,
            &paths,
            binary,
            &ffmpeg_directory,
            deno,
            &clean_url,
            &engine_version,
        ) {
            Ok(video) => {
                youtube_access::cache_analysis(&app, &video);
                videos.push(video);
            }
            Err(mut failure) => {
                failure.retry_after_epoch = youtube_access::cooldown_until(&app);
                if let Some(until) = failure.retry_after_epoch {
                    failure.message = youtube_access::cooldown_message(until);
                }
                failures.push(failure);
            }
        }
    }
    Ok(AnalysisResult { videos, failures })
}

#[tauri::command]
async fn analyze_urls(
    app: tauri::AppHandle,
    urls: Vec<String>,
    ignore_history: Option<bool>,
) -> Result<AnalysisResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        analyze_urls_blocking(app, urls, ignore_history.unwrap_or(false))
    })
    .await
    .map_err(|_| "El análisis se interrumpió inesperadamente.".to_owned())?
}

pub(crate) struct ResolvedDownloadSource {
    pub strategy: YoutubeAccessStrategy,
    pub format_id: Option<String>,
    pub format_has_audio: Option<bool>,
}

/// Re-extracts a source after an access token or format URL expires. A selected
/// height is an invariant: this helper never substitutes a lower resolution.
pub(crate) fn resolve_download_source(
    app: &tauri::AppHandle,
    url: &str,
    required_height: Option<u32>,
    excluded_format_id: Option<&str>,
) -> Result<ResolvedDownloadSource, String> {
    let paths = engine::EnginePaths::resolve(app);
    paths.required_for_youtube()?;
    let binary = paths
        .yt_dlp
        .as_ref()
        .ok_or_else(|| "yt-dlp no está disponible.".to_owned())?;
    let ffmpeg_directory = engine::yt_dlp_ffmpeg_location(app, &paths)?;
    let deno = paths
        .deno
        .as_ref()
        .ok_or_else(|| "Deno no está disponible.".to_owned())?;
    let video = analyze_url_automatically(
        app,
        &paths,
        binary,
        &ffmpeg_directory,
        deno,
        url,
        &yt_dlp_version(binary),
    )
    .map_err(|failure| failure.message)?;
    let quality = match required_height {
        Some(height) => Some(
            video
                .qualities
                .iter()
                .find(|quality| quality.height == height)
                .ok_or_else(|| format!("La fuente ya no ofrece la calidad exacta de {height}p; no se descargó una resolución inferior."))?,
        ),
        None => video.qualities.first(),
    };
    let selected_format = quality.and_then(|quality| {
        quality
            .video_formats
            .iter()
            .filter(|format| Some(format.id.as_str()) != excluded_format_id)
            .max_by(|left, right| preferred_format(left, right))
    });
    if required_height.is_some() && selected_format.is_none() {
        let height = required_height.expect("required height was checked above");
        return Err(format!(
            "No queda otra fuente utilizable para {height}p; no se descargó una resolución inferior. Elegí otra resolución y volvé a intentarlo."
        ));
    }
    Ok(ResolvedDownloadSource {
        strategy: video.access_strategy,
        format_id: selected_format.map(|format| format.id.clone()),
        format_has_audio: selected_format.map(|format| format.has_audio),
    })
}

#[tauri::command]
fn default_download_directory() -> String {
    std::env::var("USERPROFILE")
        .map(|home| format!("{home}\\Downloads"))
        .unwrap_or_else(|_| "Elegí una carpeta de destino".to_owned())
}

#[tauri::command]
fn add_download_job(
    app: tauri::AppHandle,
    request: download::DownloadRequest,
) -> Result<download::DownloadJob, String> {
    download::add(app, request)
}
#[tauri::command]
fn get_download_queue(app: tauri::AppHandle) -> download::QueueSnapshot {
    download::get_queue(app)
}
#[tauri::command]
fn start_download_queue(app: tauri::AppHandle) -> Result<(), String> {
    download::start(app)
}
#[tauri::command]
fn start_download_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    download::start_one(app, job_id)
}
#[tauri::command]
fn pause_download_queue(app: tauri::AppHandle) {
    download::pause(app)
}
#[tauri::command]
fn resume_download_queue(app: tauri::AppHandle) {
    download::resume(app)
}
#[tauri::command]
fn cancel_download_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    download::cancel(app, job_id)
}
#[tauri::command]
fn cancel_all_downloads(app: tauri::AppHandle) -> Result<(), String> {
    download::cancel_all(app)
}
#[tauri::command]
fn clear_finished_downloads(app: tauri::AppHandle) -> Result<(), String> {
    download::clear_finished(app)
}
#[tauri::command]
fn retry_download_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    download::retry(app, job_id)
}
#[tauri::command]
fn open_download_file(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    download::open_file(app, job_id)
}
#[tauri::command]
fn open_download_folder(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    download::open_folder(app, job_id)
}
#[tauri::command]
fn get_history(app: tauri::AppHandle) -> Result<Vec<history::HistoryEntry>, String> {
    history::list(&app)
}
#[tauri::command]
fn remove_history_entry(app: tauri::AppHandle, id: String) -> Result<(), String> {
    history::remove(&app, &id)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        automatic_failure, classify_youtube_failure, parse_video, youtube_failure_message,
        youtube_requires_browser_session, youtube_video_id, AnalysisAttemptFailure, BrowserSession,
        YoutubeAccessStrategy, YoutubeFailureClass,
    };
    use serde_json::json;

    #[test]
    fn exposes_downloadable_resolutions_and_excludes_super_resolution() {
        let metadata = json!({
            "id": "example",
            "title": "Example",
            "formats": [
                { "format_id": "401", "ext": "webm", "width": 3840, "height": 2160, "fps": 30, "vcodec": "av01", "acodec": "none", "tbr": 12000, "format_note": "2160p", "url": "https://example.test/2160" },
                { "format_id": "399", "ext": "mp4", "width": 1920, "height": 1080, "fps": 60, "vcodec": "avc1", "acodec": "none", "tbr": 7000, "url": "https://example.test/1080" },
                { "format_id": "247-sr", "ext": "webm", "width": 1280, "height": 720, "fps": 30, "vcodec": "vp9", "acodec": "none", "tbr": 2500, "format_note": "720p", "url": "https://example.test/720-sr" },
                { "format_id": "sb0", "ext": "mhtml", "width": 1920, "height": 1080, "vcodec": "avc1", "acodec": "none", "format_note": "storyboard", "url": "https://example.test/storyboard" },
                { "format_id": "drm", "ext": "mp4", "width": 1280, "height": 720, "vcodec": "avc1", "acodec": "none", "has_drm": true, "url": "https://example.test/drm" },
                { "format_id": "missing", "ext": "mp4", "width": 854, "height": 480, "vcodec": "avc1", "acodec": "none" }
            ]
        });
        let video = parse_video(
            "https://www.youtube.com/watch?v=example".to_owned(),
            metadata.to_string().as_bytes(),
        )
        .expect("metadata should parse");
        let heights: Vec<u32> = video
            .qualities
            .iter()
            .map(|quality| quality.height)
            .collect();
        assert_eq!(heights, vec![2160, 1080]);
        assert_eq!(video.qualities[0].format_id, "401");
        assert!(video
            .formats
            .iter()
            .all(|format| !format.id.ends_with("-sr")));
    }

    #[test]
    fn normalizes_common_video_urls_before_any_network_access() {
        let id = "eUV9noIBD8I";
        assert_eq!(
            youtube_video_id(&format!("https://www.youtube.com/watch?v={id}&t=1324s")),
            Some(id.to_owned())
        );
        assert_eq!(
            youtube_video_id(&format!("https://youtu.be/{id}?si=example")),
            Some(id.to_owned())
        );
        assert_eq!(
            youtube_video_id(&format!("https://www.youtube.com/shorts/{id}")),
            Some(id.to_owned())
        );
    }

    #[test]
    fn keeps_the_source_quality_name_for_nonstandard_frame_sizes() {
        let metadata = json!({
            "id": "cinema", "title": "Cinema",
            "formats": [{ "format_id": "398", "ext": "mp4", "width": 1280, "height": 640, "vcodec": "avc1", "acodec": "none", "format_note": "720p", "url": "https://example.test/720" }]
        });
        let video = parse_video(
            "https://www.youtube.com/watch?v=cinema".to_owned(),
            metadata.to_string().as_bytes(),
        )
        .expect("metadata should parse");
        assert_eq!(video.qualities[0].label, "720p (1280 × 640 px)");
    }

    #[test]
    fn explains_when_a_chromium_cookie_database_is_locked_without_exposing_it() {
        let message = youtube_failure_message(
            "ERROR: Could not copy Chrome cookie database. Permission denied: C:\\Users\\person\\AppData\\Local\\Microsoft\\Edge\\User Data\\Default\\Network\\Cookies",
            &YoutubeAccessStrategy::Browser { browser: BrowserSession::Edge, use_pot_provider: false },
        ).expect("known browser lock should have a safe message");
        assert!(message.contains("Edge"));
        assert!(message.contains("continuó"));
        assert!(!message.contains("C:\\Users"));
        assert!(!message.contains("Network\\Cookies"));
    }

    #[test]
    fn explains_dpapi_protection_without_disabling_encryption() {
        let message = youtube_failure_message(
            "ERROR: Failed to decrypt with DPAPI",
            &YoutubeAccessStrategy::Browser {
                browser: BrowserSession::Chrome,
                use_pot_provider: false,
            },
        )
        .expect("DPAPI failure should have a safe message");
        assert!(message.contains("Chrome"));
        assert!(message.contains("no desactivó el cifrado"));
    }

    #[test]
    fn browser_recovery_flag_means_account_not_antibot() {
        assert!(!youtube_requires_browser_session(
            "ERROR: Sign in to confirm you’re not a bot"
        ));
        assert!(youtube_requires_browser_session(
            "ERROR: This is members-only content. Join this channel"
        ));
        assert!(!youtube_requires_browser_session(
            "ERROR: Could not copy Chrome cookie database"
        ));
    }

    #[test]
    fn account_required_stops_without_reading_browser_sessions() {
        let failure = automatic_failure(
            "https://www.youtube.com/watch?v=example".to_owned(),
            &[AnalysisAttemptFailure {
                class: YoutubeFailureClass::AccountRequired,
                message: None,
            }],
        );
        assert!(failure.requires_browser_session);
        assert!(failure.message.contains("no lee ni usa sesiones"));
    }

    #[test]
    fn classifies_retryable_and_terminal_failures_structurally() {
        assert_eq!(
            classify_youtube_failure("ERROR: Sign in to confirm you're not a bot"),
            YoutubeFailureClass::AntiBotChallenge
        );
        assert_eq!(
            classify_youtube_failure("ERROR: HTTP Error 429: Too Many Requests"),
            YoutubeFailureClass::RateLimited
        );
        assert_eq!(
            classify_youtube_failure("ERROR: This video is not available in your country"),
            YoutubeFailureClass::GeoRestricted
        );
        assert_eq!(
            classify_youtube_failure("ERROR: Failed to decrypt with DPAPI"),
            YoutubeFailureClass::CookieDecryptUnsupported
        );
    }

    #[test]
    fn access_order_uses_pot_for_antibot_without_browser_cookies() {
        assert!(YoutubeFailureClass::AntiBotChallenge.warrants_pot());
        assert!(!YoutubeFailureClass::AccountRequired.warrants_pot());
    }

    #[test]
    fn serializes_the_winning_combined_strategy_for_frontend_round_trips() {
        let value = serde_json::to_value(YoutubeAccessStrategy::Browser {
            browser: BrowserSession::Firefox,
            use_pot_provider: true,
        })
        .expect("strategy should serialize");
        assert_eq!(
            value,
            json!({ "kind": "browser", "browser": "firefox", "usePotProvider": true })
        );
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        // Register this first so a second launch only raises the existing
        // window instead of creating competing queue/SQLite/provider owners.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(download::DownloadManager::default())
        .manage(engine::PotProviderManager::default())
        .manage(youtube_access::YoutubeAccessManager::default())
        .setup(|app| {
            history::initialize(app.handle()).map_err(Box::<dyn std::error::Error>::from)?;
            youtube_access::initialize(app.handle()).map_err(Box::<dyn std::error::Error>::from)?;
            // A process launched by a development host can inherit a minimized
            // show-state on Windows. Always present the main desktop window.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_engines,
            analyze_urls,
            default_download_directory,
            add_download_job,
            get_download_queue,
            start_download_queue,
            start_download_job,
            pause_download_queue,
            resume_download_queue,
            cancel_download_job,
            cancel_all_downloads,
            clear_finished_downloads,
            retry_download_job,
            open_download_file,
            open_download_folder,
            get_history,
            remove_history_entry
        ])
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
