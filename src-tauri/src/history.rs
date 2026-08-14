use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub video_id: String,
    pub url: String,
    pub title: String,
    pub thumbnail: Option<String>,
    pub channel: Option<String>,
    pub resolution: String,
    pub container: String,
    pub file_path: String,
    pub downloaded_at: i64,
}

fn database_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("No se pudo resolver el directorio de datos: {error}"))?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("No se pudo crear el directorio de datos: {error}"))?;
    Ok(directory.join("history.sqlite3"))
}

fn connection(app: &tauri::AppHandle) -> Result<Connection, String> {
    let database = Connection::open(database_path(app)?)
        .map_err(|error| format!("No se pudo abrir SQLite: {error}"))?;
    database
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|error| format!("No se pudo configurar SQLite: {error}"))?;
    database
        .execute_batch(
            "PRAGMA journal_mode = WAL;
        CREATE TABLE IF NOT EXISTS download_history (
          id TEXT PRIMARY KEY NOT NULL,
          video_id TEXT NOT NULL,
          url TEXT NOT NULL,
          title TEXT NOT NULL,
          thumbnail TEXT,
          channel TEXT,
          resolution TEXT NOT NULL,
          container TEXT NOT NULL,
          file_path TEXT NOT NULL,
          downloaded_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_download_history_downloaded_at
          ON download_history(downloaded_at DESC);
        CREATE INDEX IF NOT EXISTS idx_download_history_video_id
          ON download_history(video_id, downloaded_at DESC);",
        )
        .map_err(|error| format!("No se pudo inicializar SQLite: {error}"))?;
    Ok(database)
}

pub fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
    connection(app).map(|_| ())
}

pub fn insert(app: &tauri::AppHandle, entry: &HistoryEntry) -> Result<(), String> {
    let database = connection(app)?;
    database.execute(
        "INSERT INTO download_history
         (id, video_id, url, title, thumbnail, channel, resolution, container, file_path, downloaded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO NOTHING",
        params![entry.id, entry.video_id, entry.url, entry.title, entry.thumbnail, entry.channel, entry.resolution, entry.container, entry.file_path, entry.downloaded_at],
    ).map_err(|error| format!("No se pudo guardar el historial: {error}"))?;
    Ok(())
}

pub fn list(app: &tauri::AppHandle) -> Result<Vec<HistoryEntry>, String> {
    let database = connection(app)?;
    let mut statement = database.prepare(
        "SELECT id, video_id, url, title, thumbnail, channel, resolution, container, file_path, downloaded_at
         FROM download_history ORDER BY downloaded_at DESC",
    ).map_err(|error| format!("No se pudo consultar el historial: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                video_id: row.get(1)?,
                url: row.get(2)?,
                title: row.get(3)?,
                thumbnail: row.get(4)?,
                channel: row.get(5)?,
                resolution: row.get(6)?,
                container: row.get(7)?,
                file_path: row.get(8)?,
                downloaded_at: row.get(9)?,
            })
        })
        .map_err(|error| format!("No se pudo leer el historial: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("No se pudo leer una entrada del historial: {error}"))
}

pub fn get(app: &tauri::AppHandle, id: &str) -> Result<Option<HistoryEntry>, String> {
    let database = connection(app)?;
    let mut statement = database.prepare(
        "SELECT id, video_id, url, title, thumbnail, channel, resolution, container, file_path, downloaded_at
         FROM download_history WHERE id = ?1",
    ).map_err(|error| format!("No se pudo consultar el historial: {error}"))?;
    let mut rows = statement
        .query(params![id])
        .map_err(|error| format!("No se pudo consultar la descarga: {error}"))?;
    let Some(row) = rows
        .next()
        .map_err(|error| format!("No se pudo leer la descarga: {error}"))?
    else {
        return Ok(None);
    };
    Ok(Some(HistoryEntry {
        id: row
            .get(0)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
        video_id: row
            .get(1)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
        url: row
            .get(2)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
        title: row
            .get(3)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
        thumbnail: row
            .get(4)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
        channel: row
            .get(5)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
        resolution: row
            .get(6)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
        container: row
            .get(7)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
        file_path: row
            .get(8)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
        downloaded_at: row
            .get(9)
            .map_err(|error| format!("No se pudo leer la descarga: {error}"))?,
    }))
}

pub fn find_existing_by_video_id(
    app: &tauri::AppHandle,
    video_id: &str,
) -> Result<Option<HistoryEntry>, String> {
    let database = connection(app)?;
    let mut statement = database
        .prepare(
            "SELECT id, video_id, url, title, thumbnail, channel, resolution, container, file_path, downloaded_at
             FROM download_history WHERE video_id = ?1 ORDER BY downloaded_at DESC",
        )
        .map_err(|error| format!("No se pudo consultar el historial: {error}"))?;
    let rows = statement
        .query_map(params![video_id], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                video_id: row.get(1)?,
                url: row.get(2)?,
                title: row.get(3)?,
                thumbnail: row.get(4)?,
                channel: row.get(5)?,
                resolution: row.get(6)?,
                container: row.get(7)?,
                file_path: row.get(8)?,
                downloaded_at: row.get(9)?,
            })
        })
        .map_err(|error| format!("No se pudo consultar el historial: {error}"))?;
    for entry in rows {
        let entry = entry.map_err(|error| format!("No se pudo leer una descarga: {error}"))?;
        if std::path::Path::new(&entry.file_path).is_file() {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

pub fn remove(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let database = connection(app)?;
    database
        .execute("DELETE FROM download_history WHERE id = ?1", params![id])
        .map_err(|error| format!("No se pudo eliminar la entrada del historial: {error}"))?;
    Ok(())
}
