use crate::{engine, hidden_command, history};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use url::Url;

const PROGRESS_PREFIX: &str = "__YTDM_PROGRESS__";
const PROCESSING_PREFIX: &str = "__YTDM_POSTPROCESS__";
const FILE_PREFIX: &str = "__YTDM_FILE__";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus { Pending, Analyzing, Ready, Queued, Downloading, Processing, Completed, Failed, Cancelled }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress { pub percent: Option<f64>, pub speed: Option<f64>, pub eta: Option<f64>, pub downloaded_bytes: u64, pub total_bytes: Option<u64> }
impl Default for DownloadProgress { fn default() -> Self { Self { percent: None, speed: None, eta: None, downloaded_bytes: 0, total_bytes: None } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadVerification { pub width: Option<u32>, pub height: Option<u32>, pub duration: Option<f64>, pub video_codec: Option<String>, pub audio_codec: Option<String> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    pub video_id: String, pub url: String, pub title: String, pub thumbnail: Option<String>, pub channel: Option<String>,
    pub quality_height: Option<u32>, pub selected_format_id: Option<String>, pub selected_format_has_audio: Option<bool>, #[serde(default)] pub compatibility_mode: bool, #[serde(default)] pub browser_session: Option<String>, pub container: String, pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJob {
    #[serde(flatten)] pub request: DownloadRequest,
    pub job_id: String, pub status: DownloadStatus, pub progress: DownloadProgress, pub message: Option<String>, pub error: Option<String>,
    pub file_path: Option<String>, pub created_at: Option<String>, pub completed_at: Option<String>, pub verification: Option<DownloadVerification>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadEvent { pub job_id: String, pub job: Option<DownloadJob>, pub progress: Option<DownloadProgress>, pub message: Option<String>, pub error: Option<String> }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot { pub jobs: Vec<DownloadJob>, pub is_paused: bool }

#[derive(Default)]
struct QueueState { jobs: Vec<DownloadJob>, paused: bool, worker_running: bool }

#[derive(Default)]
pub struct DownloadManager { state: Mutex<QueueState>, processes: Mutex<HashMap<String, u32>> }

fn now_string() -> String { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().to_string() }
fn now_epoch() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }
fn job_id() -> String { format!("job-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()) }

fn valid_container(value: &str) -> bool { matches!(value, "auto" | "mp4" | "mkv" | "webm") }
fn valid_format_id(value: &str) -> bool { !value.is_empty() && value.len() <= 96 && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')) }
fn valid_browser_session(value: Option<&str>) -> bool { matches!(value, None | Some("chrome") | Some("edge")) }
fn valid_youtube_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value.trim()) else { return false; };
    if !matches!(url.scheme(), "https" | "http") { return false; }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else { return false; };
    host == "youtu.be" || host == "www.youtu.be" || host == "youtube.com" || host.ends_with(".youtube.com")
}
fn paths_ready(app: &AppHandle) -> Result<engine::EnginePaths, String> { let paths = engine::EnginePaths::resolve(app); paths.required_for_youtube()?; Ok(paths) }

fn snapshot(app: &AppHandle) -> QueueSnapshot {
    let manager = app.state::<DownloadManager>();
    let state = manager.state.lock().expect("queue lock poisoned");
    QueueSnapshot { jobs: state.jobs.clone(), is_paused: state.paused }
}
fn emit_queue(app: &AppHandle) { let _ = app.emit("queue://updated", snapshot(app)); }
fn find_job(app: &AppHandle, job_id: &str) -> Option<DownloadJob> {
    app.state::<DownloadManager>().state.lock().ok()?.jobs.iter().find(|job| job.job_id == job_id).cloned()
}
fn update_job<F>(app: &AppHandle, job_id: &str, update: F) -> Option<DownloadJob> where F: FnOnce(&mut DownloadJob) {
    let manager = app.state::<DownloadManager>();
    let mut state = manager.state.lock().ok()?;
    let job = state.jobs.iter_mut().find(|job| job.job_id == job_id)?;
    update(job);
    Some(job.clone())
}
fn emit_job(app: &AppHandle, event: &str, job_id: &str, include_job: bool, progress: Option<DownloadProgress>, message: Option<String>, error: Option<String>) {
    let job = include_job.then(|| find_job(app, job_id)).flatten();
    let _ = app.emit(event, DownloadEvent { job_id: job_id.to_owned(), job, progress, message, error });
    emit_queue(app);
}

pub fn add(app: AppHandle, request: DownloadRequest) -> Result<DownloadJob, String> {
    paths_ready(&app)?;
    if !valid_container(&request.container) { return Err("Contenedor no válido.".to_owned()); }
    if request.selected_format_id.as_deref().is_some_and(|format_id| !valid_format_id(format_id)) { return Err("El formato seleccionado no es válido.".to_owned()); }
    if !valid_browser_session(request.browser_session.as_deref()) { return Err("El navegador seleccionado no es compatible.".to_owned()); }
    if request.video_id.trim().is_empty() || request.url.trim().is_empty() || request.destination.trim().is_empty() { return Err("Faltan datos para agregar la descarga a la cola.".to_owned()); }
    if !valid_youtube_url(&request.url) { return Err("La URL no es un enlace de YouTube válido.".to_owned()); }
    if app.state::<DownloadManager>().state.lock().map_err(|_| "No se pudo bloquear la cola.".to_owned())?
        .jobs.iter().any(|existing| existing.request.video_id == request.video_id && !matches!(existing.status, DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled)) {
        return Err("Ese video ya está en la cola.".to_owned());
    }
    let job = DownloadJob { request, job_id: job_id(), status: DownloadStatus::Queued, progress: DownloadProgress::default(), message: Some("En espera".to_owned()), error: None, file_path: None, created_at: Some(now_string()), completed_at: None, verification: None };
    app.state::<DownloadManager>().state.lock().map_err(|_| "No se pudo bloquear la cola.".to_owned())?.jobs.push(job.clone());
    emit_queue(&app);
    Ok(job)
}
pub fn get_queue(app: AppHandle) -> QueueSnapshot { snapshot(&app) }
pub fn pause(app: AppHandle) { if let Ok(mut state) = app.state::<DownloadManager>().state.lock() { state.paused = true; } emit_queue(&app); }
pub fn resume(app: AppHandle) { if let Ok(mut state) = app.state::<DownloadManager>().state.lock() { state.paused = false; } emit_queue(&app); let _ = start(app); }

pub fn start(app: AppHandle) -> Result<(), String> {
    paths_ready(&app)?;
    let should_start = {
        let manager = app.state::<DownloadManager>();
        let mut state = manager.state.lock().map_err(|_| "No se pudo bloquear la cola.".to_owned())?;
        state.paused = false;
        if state.worker_running { false } else { state.worker_running = true; true }
    };
    emit_queue(&app);
    if should_start { thread::spawn(move || run_queue(app)); }
    Ok(())
}

/// Runs exactly one queued item and leaves every other queued item waiting.
/// This is deliberately separate from `start`, which drains the full queue.
pub fn start_one(app: AppHandle, job_id: String) -> Result<(), String> {
    paths_ready(&app)?;
    let job = {
        let manager = app.state::<DownloadManager>();
        let mut state = manager.state.lock().map_err(|_| "No se pudo bloquear la cola.".to_owned())?;
        if state.worker_running {
            return Err("Ya hay una descarga en curso.".to_owned());
        }
        let position = state.jobs.iter().position(|job| job.job_id == job_id)
            .ok_or_else(|| "No se encontró la descarga.".to_owned())?;
        if !matches!(state.jobs[position].status, DownloadStatus::Queued | DownloadStatus::Pending) {
            return Err("Solo se puede iniciar una descarga que está en espera.".to_owned());
        }
        state.paused = false;
        state.worker_running = true;
        let job = &mut state.jobs[position];
        job.status = DownloadStatus::Downloading;
        job.message = Some("Iniciando yt-dlp…".to_owned());
        job.error = None;
        job.clone()
    };

    emit_queue(&app);
    thread::spawn(move || {
        run_job(&app, &job);
        if let Ok(mut state) = app.state::<DownloadManager>().state.lock() {
            state.worker_running = false;
        }
        emit_queue(&app);
    });
    Ok(())
}

fn next_job(app: &AppHandle) -> Option<DownloadJob> {
    let manager = app.state::<DownloadManager>();
    let mut state = manager.state.lock().ok()?;
    if state.paused { state.worker_running = false; return None; }
    let Some(job) = state.jobs.iter_mut().find(|job| matches!(job.status, DownloadStatus::Queued | DownloadStatus::Pending)) else {
        state.worker_running = false;
        return None;
    };
    job.status = DownloadStatus::Downloading;
    job.message = Some("Iniciando yt-dlp…".to_owned());
    job.error = None;
    Some(job.clone())
}
fn run_queue(app: AppHandle) {
    loop {
        let Some(job) = next_job(&app) else { emit_queue(&app); break; };
        run_job(&app, &job);
    }
}

fn run_job(app: &AppHandle, job: &DownloadJob) {
    emit_job(app, "download://started", &job.job_id, true, None, Some("Descargando…".to_owned()), None);
    if let Err(error) = execute_download(app, job) {
        let cancelled = find_job(app, &job.job_id).is_some_and(|current| current.status == DownloadStatus::Cancelled);
        if !cancelled {
            update_job(app, &job.job_id, |current| { current.status = DownloadStatus::Failed; current.error = Some(error.clone()); current.message = Some("La descarga falló".to_owned()); });
            emit_job(app, "download://failed", &job.job_id, true, None, None, Some(error));
        }
    }
}

fn parse_number(value: Option<&str>) -> Option<f64> { value?.trim().parse::<f64>().ok() }
fn parse_bytes(value: Option<&str>) -> Option<u64> { value?.trim().parse::<u64>().ok() }
fn parse_progress(line: &str) -> Option<DownloadProgress> {
    let payload = line.strip_prefix(PROGRESS_PREFIX)?;
    let fields: Vec<&str> = payload.split('\t').collect();
    if fields.len() < 7 { return None; }
    let percent = fields[1].trim().trim_end_matches('%').trim().parse::<f64>().ok();
    let total = parse_bytes(fields.get(3).copied()).or_else(|| parse_bytes(fields.get(4).copied()));
    Some(DownloadProgress { percent, downloaded_bytes: parse_bytes(fields.get(2).copied()).unwrap_or(0), total_bytes: total, speed: parse_number(fields.get(5).copied()), eta: parse_number(fields.get(6).copied()) })
}
fn dispatch_line(app: &AppHandle, job_id: &str, line: &str, final_path: &Arc<Mutex<Option<PathBuf>>>) {
    if let Some(progress) = parse_progress(line) {
        update_job(app, job_id, |job| { job.progress = progress.clone(); job.message = Some("Descargando…".to_owned()); });
        emit_job(app, "download://progress", job_id, false, Some(progress), None, None);
    } else if line.starts_with(PROCESSING_PREFIX) {
        update_job(app, job_id, |job| { job.status = DownloadStatus::Processing; job.message = Some("Fusionando video + audio…".to_owned()); });
        emit_job(app, "download://processing", job_id, true, None, Some("Fusionando video + audio…".to_owned()), None);
    } else if let Some(json_path) = line.strip_prefix(FILE_PREFIX) {
        if let Ok(path) = serde_json::from_str::<String>(json_path.trim()) { if let Ok(mut target) = final_path.lock() { *target = Some(PathBuf::from(path)); } }
    }
}

fn read_stream<R: std::io::Read + Send + 'static>(stream: R, app: AppHandle, job_id: String, final_path: Arc<Mutex<Option<PathBuf>>>, diagnostics: Arc<Mutex<String>>) -> thread::JoinHandle<()> {
    thread::spawn(move || for line in BufReader::new(stream).lines().map_while(Result::ok) {
        dispatch_line(&app, &job_id, &line, &final_path);
        if !line.starts_with(PROGRESS_PREFIX) && !line.starts_with(PROCESSING_PREFIX) && !line.starts_with(FILE_PREFIX) { if let Ok(mut log) = diagnostics.lock() { log.push_str(&line); log.push('\n'); } }
    })
}

fn output_directory(job: &DownloadJob) -> Result<PathBuf, String> {
    let directory = PathBuf::from(&job.request.destination);
    if !directory.is_absolute() { return Err("La carpeta de destino debe ser absoluta.".to_owned()); }
    fs::create_dir_all(&directory).map_err(|error| format!("No se pudo crear la carpeta de destino: {error}"))?;
    directory.canonicalize().map_err(|error| format!("No se pudo acceder a la carpeta de destino: {error}"))
}
fn requested_format_selector(selected_format_id: Option<&str>, selected_format_has_audio: Option<bool>, quality_height: Option<u32>, compatibility_mode: bool) -> String {
    if let Some(format_id) = selected_format_id {
        return if selected_format_has_audio == Some(true) { format_id.to_owned() } else { format!("{format_id}+ba") };
    }
    if compatibility_mode { return "bv*+ba/b".to_owned(); }
    match quality_height { Some(height) => format!("bv*[height={height}]+ba"), None => "bv*+ba/b".to_owned() }
}
fn format_selector(job: &DownloadJob) -> String {
    requested_format_selector(job.request.selected_format_id.as_deref(), job.request.selected_format_has_audio, job.request.quality_height, job.request.compatibility_mode)
}

fn execute_download(app: &AppHandle, job: &DownloadJob) -> Result<(), String> {
    let paths = paths_ready(app)?;
    let destination = output_directory(job)?;
    let binary = paths.yt_dlp.as_ref().ok_or_else(|| "yt-dlp no está disponible.".to_owned())?;
    let ffmpeg_directory = engine::yt_dlp_ffmpeg_location(app, &paths)?;
    let deno = paths.deno.ok_or_else(|| "Deno no está disponible.".to_owned())?;
    let mut command = hidden_command(binary);
    command.args(["--ignore-config", "--no-plugin-dirs", "--no-playlist", "--newline", "--progress", "--windows-filenames", "--trim-filenames", "180", "--no-overwrites", "--paths"])
        .arg(&destination).arg("--output").arg("%(title)s [%(height)sp].%(ext)s")
        .arg("--ffmpeg-location").arg(ffmpeg_directory).arg("--js-runtimes").arg(format!("deno:{}", deno.display()));
    if let Some(browser) = job.request.browser_session.as_deref() {
        command.arg("--cookies-from-browser").arg(browser)
            .arg("--extractor-args").arg("youtube:player_client=web_safari");
    } else {
        command.arg("--extractor-args").arg("youtube:player_client=android_vr");
    }
    command
        .arg("--format").arg(format_selector(job))
        .arg("--progress-template").arg(format!("download:{PROGRESS_PREFIX}%(progress.status)s\t%(progress._percent_str)s\t%(progress.downloaded_bytes)s\t%(progress.total_bytes)s\t%(progress.total_bytes_estimate)s\t%(progress.speed)s\t%(progress.eta)s"))
        .arg("--progress-template").arg(format!("postprocess:{PROCESSING_PREFIX}%(progress.status)s"))
        .arg("--print").arg(format!("after_move:{FILE_PREFIX}%(filepath)j"))
        .arg(&job.request.url).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if job.request.container != "auto" { command.arg("--merge-output-format").arg(&job.request.container); }
    let mut child = command.spawn().map_err(|error| format!("No se pudo iniciar yt-dlp: {error}"))?;
    let process_id = child.id();
    let stdout = child.stdout.take().ok_or_else(|| "No se pudo leer el progreso de yt-dlp.".to_owned())?;
    let stderr = child.stderr.take().ok_or_else(|| "No se pudo leer los errores de yt-dlp.".to_owned())?;
    app.state::<DownloadManager>().processes.lock().map_err(|_| "No se pudo registrar el proceso de descarga.".to_owned())?.insert(job.job_id.clone(), process_id);
    let final_path = Arc::new(Mutex::new(None));
    let diagnostics = Arc::new(Mutex::new(String::new()));
    let stdout_reader = read_stream(stdout, app.clone(), job.job_id.clone(), final_path.clone(), diagnostics.clone());
    let stderr_reader = read_stream(stderr, app.clone(), job.job_id.clone(), final_path.clone(), diagnostics.clone());
    let exit = child.wait().map_err(|error| format!("yt-dlp terminó inesperadamente: {error}"))?;
    let _ = stdout_reader.join(); let _ = stderr_reader.join();
    app.state::<DownloadManager>().processes.lock().ok().and_then(|mut processes| processes.remove(&job.job_id));
    if find_job(app, &job.job_id).is_some_and(|current| current.status == DownloadStatus::Cancelled) { return Err("Descarga cancelada.".to_owned()); }
    if !exit.success() { return Err(diagnostics.lock().ok().map(|log| log.trim().to_owned()).filter(|text| !text.is_empty()).unwrap_or_else(|| "yt-dlp no pudo completar la descarga.".to_owned())); }
    let file = final_path.lock().ok().and_then(|path| path.clone()).ok_or_else(|| "yt-dlp no informó la ruta final del archivo.".to_owned())?;
    let canonical_file = file.canonicalize().map_err(|error| format!("No se pudo localizar el archivo final: {error}"))?;
    if !canonical_file.starts_with(&destination) { return Err("La ruta final no pertenece a la carpeta seleccionada.".to_owned()); }
    let verification = verify_file(app, &canonical_file)?;
    if verification.video_codec.is_none() || verification.audio_codec.is_none() { return Err("El archivo final no contiene video y audio válidos.".to_owned()); }
    if let Some(height) = job.request.quality_height { if verification.height != Some(height) { return Err(format!("La verificación final informó {}p, no la calidad solicitada de {height}p.", verification.height.unwrap_or(0))); } }
    let file_string = canonical_file.display().to_string();
    update_job(app, &job.job_id, |current| { current.status = DownloadStatus::Completed; current.message = Some("Completado".to_owned()); current.file_path = Some(file_string.clone()); current.verification = Some(verification.clone()); current.completed_at = Some(now_string()); current.progress.percent = Some(100.0); });
    let completed = find_job(app, &job.job_id).ok_or_else(|| "No se encontró el trabajo completado.".to_owned())?;
    if let Err(error) = history::insert(app, &history::HistoryEntry { id: completed.job_id.clone(), video_id: completed.request.video_id.clone(), url: completed.request.url.clone(), title: completed.request.title.clone(), thumbnail: completed.request.thumbnail.clone(), channel: completed.request.channel.clone(), resolution: completed.request.quality_height.map(|height| format!("{height}p")).unwrap_or_else(|| "Mejor calidad".to_owned()), container: completed.request.container.clone(), file_path: file_string, downloaded_at: now_epoch() }) {
        update_job(app, &job.job_id, |current| current.message = Some(format!("Completado; historial no guardado: {error}")));
    }
    emit_job(app, "download://completed", &job.job_id, true, None, Some("Completado".to_owned()), None);
    Ok(())
}

fn verify_file(app: &AppHandle, file: &Path) -> Result<DownloadVerification, String> {
    let ffprobe = engine::EnginePaths::resolve(app).ffprobe.ok_or_else(|| "ffprobe no está disponible.".to_owned())?;
    let output = hidden_command(ffprobe).args(["-v", "error", "-show_streams", "-show_format", "-of", "json"]).arg(file).output().map_err(|error| format!("No se pudo iniciar ffprobe: {error}"))?;
    if !output.status.success() { return Err("ffprobe no pudo validar el archivo final.".to_owned()); }
    let data: Value = serde_json::from_slice(&output.stdout).map_err(|_| "ffprobe devolvió datos inválidos.".to_owned())?;
    let streams = data.get("streams").and_then(Value::as_array).ok_or_else(|| "ffprobe no encontró streams.".to_owned())?;
    let video = streams.iter().find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("video"));
    let audio = streams.iter().find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"));
    Ok(DownloadVerification { width: video.and_then(|stream| stream.get("width")).and_then(Value::as_u64).map(|value| value as u32), height: video.and_then(|stream| stream.get("height")).and_then(Value::as_u64).map(|value| value as u32), duration: data.get("format").and_then(|format| format.get("duration")).and_then(Value::as_str).and_then(|value| value.parse().ok()), video_codec: video.and_then(|stream| stream.get("codec_name")).and_then(Value::as_str).map(ToOwned::to_owned), audio_codec: audio.and_then(|stream| stream.get("codec_name")).and_then(Value::as_str).map(ToOwned::to_owned) })
}

fn terminate_process(process_id: u32) {
    let _ = hidden_command("taskkill.exe").args(["/PID", &process_id.to_string(), "/T", "/F"]).output();
}
pub fn cancel(app: AppHandle, job_id: String) -> Result<(), String> {
    let process = app.state::<DownloadManager>().processes.lock().map_err(|_| "No se pudo acceder al proceso.".to_owned())?.get(&job_id).cloned();
    let job = update_job(&app, &job_id, |current| { if !matches!(current.status, DownloadStatus::Completed) { current.status = DownloadStatus::Cancelled; current.message = Some("Cancelado".to_owned()); } }).ok_or_else(|| "No se encontró la descarga.".to_owned())?;
    if let Some(process_id) = process { terminate_process(process_id); }
    emit_job(&app, "download://cancelled", &job.job_id, true, None, Some("Cancelado".to_owned()), None);
    Ok(())
}
pub fn cancel_all(app: AppHandle) -> Result<(), String> {
    let ids: Vec<String> = snapshot(&app).jobs.into_iter().filter(|job| matches!(job.status, DownloadStatus::Queued | DownloadStatus::Pending | DownloadStatus::Downloading | DownloadStatus::Processing)).map(|job| job.job_id).collect();
    for id in ids { let _ = cancel(app.clone(), id); }
    pause(app); Ok(())
}
pub fn clear_finished(app: AppHandle) -> Result<(), String> {
    let manager = app.state::<DownloadManager>();
    {
        let mut state = manager.state.lock().map_err(|_| "No se pudo bloquear la cola.".to_owned())?;
        if state.worker_running || state.jobs.iter().any(|job| matches!(job.status, DownloadStatus::Queued | DownloadStatus::Pending | DownloadStatus::Downloading | DownloadStatus::Processing)) {
            return Err("Esperá a que terminen o cancelá las descargas pendientes antes de limpiar la sesión.".to_owned());
        }
        state.jobs.retain(|job| !matches!(job.status, DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Cancelled));
    }
    emit_queue(&app);
    Ok(())
}
pub fn retry(app: AppHandle, job_id: String) -> Result<(), String> {
    let should_resume = {
        let manager = app.state::<DownloadManager>();
        let mut state = manager.state.lock().map_err(|_| "No se pudo bloquear la cola.".to_owned())?;
        let job = state.jobs.iter_mut().find(|job| job.job_id == job_id).ok_or_else(|| "No se encontró la descarga.".to_owned())?;
        if !matches!(job.status, DownloadStatus::Failed | DownloadStatus::Cancelled) {
            return Err("Solo se pueden reintentar descargas canceladas o fallidas.".to_owned());
        }
        job.status = DownloadStatus::Queued;
        job.error = None;
        job.message = Some("En espera".to_owned());
        job.progress = DownloadProgress::default();
        job.file_path = None;
        job.verification = None;
        !state.paused && !state.worker_running
    };
    emit_queue(&app);
    if should_resume { start(app)?; }
    Ok(())
}
fn safe_job_path(app: &AppHandle, job_id: &str) -> Result<PathBuf, String> {
    if let Some(job) = find_job(app, job_id) {
        let path = job.file_path.ok_or_else(|| "La descarga aún no tiene un archivo final.".to_owned())?;
        let canonical = PathBuf::from(path).canonicalize().map_err(|_| "El archivo descargado ya no existe.".to_owned())?;
        let destination = PathBuf::from(job.request.destination).canonicalize().map_err(|_| "La carpeta de destino ya no existe.".to_owned())?;
        if !canonical.starts_with(destination) { return Err("La ruta del archivo no es válida.".to_owned()); }
        return Ok(canonical);
    }

    // After an app restart the in-memory queue is empty. The history record is
    // trusted because it is inserted only after canonical-path and ffprobe
    // verification in execute_download; the frontend still supplies only an ID.
    let entry = history::get(app, job_id)?.ok_or_else(|| "No se encontró la descarga.".to_owned())?;
    let path = PathBuf::from(entry.file_path);
    if !path.is_absolute() { return Err("La ruta guardada no es válida.".to_owned()); }
    let canonical = path.canonicalize().map_err(|_| "El archivo descargado ya no existe.".to_owned())?;
    if !canonical.is_file() { return Err("La ruta guardada no apunta a un archivo.".to_owned()); }
    Ok(canonical)
}
pub fn open_file(app: AppHandle, job_id: String) -> Result<(), String> { let path = safe_job_path(&app, &job_id)?; app.opener().open_path(path.display().to_string(), None::<&str>).map_err(|error| format!("No se pudo abrir el archivo: {error}")) }
pub fn open_folder(app: AppHandle, job_id: String) -> Result<(), String> { let path = safe_job_path(&app, &job_id)?; app.opener().reveal_item_in_dir(path).map_err(|error| format!("No se pudo abrir la carpeta: {error}")) }

#[cfg(test)]
mod tests {
    use super::{parse_progress, requested_format_selector, valid_youtube_url, PROGRESS_PREFIX};

    #[test]
    fn parses_only_structured_progress_fields() {
        let line = format!("{PROGRESS_PREFIX}downloading\t12.5%\t123\t456\tNA\t789.25\t10");
        let progress = parse_progress(&line).expect("structured yt-dlp progress should parse");
        assert_eq!(progress.percent, Some(12.5));
        assert_eq!(progress.downloaded_bytes, 123);
        assert_eq!(progress.total_bytes, Some(456));
        assert_eq!(progress.speed, Some(789.25));
        assert_eq!(progress.eta, Some(10.0));
        assert!(parse_progress("[download] 12.5% of 1MiB").is_none());
    }

    #[test]
    fn accepts_only_youtube_hosts() {
        assert!(valid_youtube_url("https://www.youtube.com/watch?v=abc"));
        assert!(valid_youtube_url("https://youtu.be/abc"));
        assert!(!valid_youtube_url("https://youtube.com.evil.example/watch?v=abc"));
        assert!(!valid_youtube_url("file:///C:/video.mp4"));
    }

    #[test]
    fn selected_format_never_falls_back_to_a_lower_resolution() {
        assert_eq!(requested_format_selector(Some("401"), Some(false), Some(2160), false), "401+ba");
        assert_eq!(requested_format_selector(Some("22"), Some(true), Some(720), false), "22");
    }

    #[test]
    fn compatibility_mode_is_explicit_and_never_used_for_a_source_selection() {
        assert_eq!(requested_format_selector(None, None, None, true), "bv*+ba/b");
        assert_eq!(requested_format_selector(Some("401"), Some(false), Some(2160), true), "401+ba");
    }

    #[test]
    fn accepts_only_supported_local_browser_sessions() {
        assert!(super::valid_browser_session(None));
        assert!(super::valid_browser_session(Some("chrome")));
        assert!(super::valid_browser_session(Some("edge")));
        assert!(!super::valid_browser_session(Some("firefox")));
    }

}
