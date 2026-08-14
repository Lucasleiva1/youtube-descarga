use crate::AnalyzedVideo;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    process::Command,
    sync::{Condvar, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

const MIN_OPERATION_GAP: Duration = Duration::from_secs(8);
const ANALYSIS_CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const BLOCK_COOLDOWN_SECONDS: u64 = 30 * 60;

#[derive(Default)]
struct TrafficState {
    active: bool,
    not_before: Option<Instant>,
    cooldown_until_epoch: Option<u64>,
}

#[derive(Clone)]
struct CachedAnalysis {
    stored_at: Instant,
    video: AnalyzedVideo,
}

#[derive(Default)]
pub struct YoutubeAccessManager {
    traffic: Mutex<TrafficState>,
    traffic_changed: Condvar,
    analysis_cache: Mutex<HashMap<String, CachedAnalysis>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAccessState {
    cooldown_until_epoch: u64,
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn state_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("No se pudo resolver el estado de acceso: {error}"))?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("No se pudo preparar el estado de acceso: {error}"))?;
    Ok(directory.join("youtube-access-state.json"))
}

pub fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
    let path = state_path(app)?;
    let cooldown = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PersistedAccessState>(&bytes).ok())
        .map(|state| state.cooldown_until_epoch)
        .filter(|until| *until > now_epoch());
    let manager = app.state::<YoutubeAccessManager>();
    manager
        .traffic
        .lock()
        .map_err(|_| "No se pudo inicializar el control de solicitudes.".to_owned())?
        .cooldown_until_epoch = cooldown;
    Ok(())
}

pub fn configure_extraction_pacing(command: &mut Command) {
    command.args([
        "--sleep-requests",
        "1.5",
        "--extractor-retries",
        "1",
        "--retry-sleep",
        "extractor:8",
    ]);
}

pub fn begin_network_operation(app: &tauri::AppHandle) -> Result<(), String> {
    let manager = app.state::<YoutubeAccessManager>();
    let mut state = manager
        .traffic
        .lock()
        .map_err(|_| "No se pudo coordinar el acceso a YouTube.".to_owned())?;
    loop {
        let current_epoch = now_epoch();
        if let Some(until) = state.cooldown_until_epoch {
            if until > current_epoch {
                return Err(cooldown_message(until));
            }
            state.cooldown_until_epoch = None;
        }
        if state.active {
            state = manager
                .traffic_changed
                .wait(state)
                .map_err(|_| "No se pudo coordinar el acceso a YouTube.".to_owned())?;
            continue;
        }
        if let Some(not_before) = state.not_before {
            let now = Instant::now();
            if not_before > now {
                let (next_state, _) = manager
                    .traffic_changed
                    .wait_timeout(state, not_before - now)
                    .map_err(|_| "No se pudo coordinar el acceso a YouTube.".to_owned())?;
                state = next_state;
                continue;
            }
        }
        state.active = true;
        return Ok(());
    }
}

pub fn finish_network_operation(app: &tauri::AppHandle) {
    let manager = app.state::<YoutubeAccessManager>();
    if let Ok(mut state) = manager.traffic.lock() {
        state.active = false;
        state.not_before = Some(Instant::now() + MIN_OPERATION_GAP);
        manager.traffic_changed.notify_all();
    };
}

pub fn cooldown_until(app: &tauri::AppHandle) -> Option<u64> {
    let manager = app.state::<YoutubeAccessManager>();
    let state = manager.traffic.lock().ok()?;
    state
        .cooldown_until_epoch
        .filter(|until| *until > now_epoch())
}

pub fn wait_for_cooldown(app: &tauri::AppHandle) {
    let manager = app.state::<YoutubeAccessManager>();
    let Ok(mut state) = manager.traffic.lock() else {
        return;
    };
    loop {
        let Some(until) = state.cooldown_until_epoch else {
            return;
        };
        let remaining = until.saturating_sub(now_epoch());
        if remaining == 0 {
            state.cooldown_until_epoch = None;
            return;
        }
        let Ok((next_state, _)) = manager
            .traffic_changed
            .wait_timeout(state, Duration::from_secs(remaining))
        else {
            return;
        };
        state = next_state;
    }
}

pub fn activate_cooldown(app: &tauri::AppHandle, reason: &str) -> u64 {
    let until = now_epoch() + BLOCK_COOLDOWN_SECONDS;
    let manager = app.state::<YoutubeAccessManager>();
    if let Ok(mut state) = manager.traffic.lock() {
        state.cooldown_until_epoch = Some(
            state
                .cooldown_until_epoch
                .map_or(until, |existing| existing.max(until)),
        );
        manager.traffic_changed.notify_all();
    }
    if let Ok(path) = state_path(app) {
        if let Ok(bytes) = serde_json::to_vec(&PersistedAccessState {
            cooldown_until_epoch: until,
        }) {
            let _ = fs::write(path, bytes);
        }
    }
    crate::engine::diagnostic_log(
        app,
        &format!("component=traffic action=cooldown reason={reason} duration_seconds={BLOCK_COOLDOWN_SECONDS}"),
    );
    until
}

pub fn cooldown_message(until: u64) -> String {
    let remaining = until.saturating_sub(now_epoch());
    let minutes = remaining.div_ceil(60).max(1);
    format!(
        "YouTube aplicó una verificación temporal. Para no agravarla, la aplicación detuvo las solicitudes durante {minutes} minuto{}.",
        if minutes == 1 { "" } else { "s" }
    )
}

pub fn cached_analysis(
    app: &tauri::AppHandle,
    video_id: &str,
    requested_url: &str,
) -> Option<AnalyzedVideo> {
    let manager = app.state::<YoutubeAccessManager>();
    let mut cache = manager.analysis_cache.lock().ok()?;
    cache.retain(|_, entry| entry.stored_at.elapsed() < ANALYSIS_CACHE_TTL);
    let mut video = cache.get(video_id)?.video.clone();
    video.url = requested_url.to_owned();
    Some(video)
}

pub fn cache_analysis(app: &tauri::AppHandle, video: &AnalyzedVideo) {
    let manager = app.state::<YoutubeAccessManager>();
    if let Ok(mut cache) = manager.analysis_cache.lock() {
        cache.insert(
            video.id.clone(),
            CachedAnalysis {
                stored_at: Instant::now(),
                video: video.clone(),
            },
        );
    };
}

#[cfg(test)]
mod tests {
    use super::configure_extraction_pacing;
    use std::process::Command;

    #[test]
    fn applies_conservative_extraction_delays_and_bounded_retries() {
        let mut command = Command::new("yt-dlp");
        configure_extraction_pacing(&mut command);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--sleep-requests", "1.5"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--extractor-retries", "1"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--retry-sleep", "extractor:8"]));
    }
}
