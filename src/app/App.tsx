import { open } from "@tauri-apps/plugin-dialog";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { check, type Update } from "@tauri-apps/plugin-updater";
import {
  CheckCircle2,
  ChevronDown,
  Clipboard,
  Clock3,
  Download,
  FileText,
  FolderOpen,
  History,
  Info,
  LoaderCircle,
  Pause,
  Play,
  Plus,
  RotateCcw,
  Search,
  Settings,
  Trash2,
  XCircle,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import logoImage from "../assets/yt-downloader-app-icon.png";
import { formatBytes, formatDuration, formatEta, formatSpeed, qualityLabel } from "../lib/formatters";
import { useDownloadStore } from "../stores/downloadStore";
import type {
  AddDownloadJobRequest,
  AnalysisFailure,
  AnalysisResult,
  AnalyzedVideo,
  BrowserSession,
  DownloadContainer,
  DownloadEvent,
  DownloadJob,
  DownloadSelection,
  DownloadStatus,
  EngineInfo,
  HistoryEntry,
  QueueSnapshot,
} from "../types/download";

const defaultDestination = "Elegí una carpeta de destino";
type YoutubeAccessMode = "public" | BrowserSession;
const youtubeAccessModeStorageKey = "yt-download.youtube-access-mode";

function savedYoutubeAccessMode(): YoutubeAccessMode {
  try {
    const value = window.localStorage.getItem(youtubeAccessModeStorageKey);
    return value === "chrome" || value === "edge" ? value : "public";
  } catch {
    return "public";
  }
}
function defaultSelectionFor(video: AnalyzedVideo): DownloadSelection {
  const highestQuality = video.qualities[0];
  return {
    qualityHeight: highestQuality?.height ?? null,
    selectedFormatId: highestQuality?.formatId ?? null,
    selectedFormatHasAudio: highestQuality?.formatHasAudio ?? null,
    compatibilityMode: false,
    container: "mp4",
  };
}

function errorMessage(reason: unknown, fallback: string): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error && reason.message) return reason.message;
  return fallback;
}

function isActiveStatus(status: DownloadStatus): boolean {
  return status === "downloading" || status === "processing";
}

function isWaitingStatus(status: DownloadStatus): boolean {
  return status === "pending" || status === "analyzing" || status === "ready" || status === "queued";
}

function statusLabel(status: DownloadStatus): string {
  switch (status) {
    case "pending":
    case "analyzing":
    case "ready":
    case "queued":
      return "EN ESPERA";
    case "downloading":
      return "DESCARGANDO";
    case "processing":
      return "PROCESANDO";
    case "completed":
      return "COMPLETADO";
    case "failed":
      return "ERROR";
    case "cancelled":
      return "CANCELADO";
  }
}

function jobQualityLabel(job: DownloadJob): string {
  return job.qualityHeight === null ? "Mejor calidad" : qualityLabel(job.qualityHeight);
}

function formatHistoryDate(timestamp: number): string {
  if (!Number.isFinite(timestamp) || timestamp <= 0) return "Fecha no disponible";
  return new Intl.DateTimeFormat("es-AR", { dateStyle: "short", timeStyle: "short" }).format(new Date(timestamp * 1000));
}

function EngineBadge({ engine }: { engine: EngineInfo }) {
  const active = engine.state === "available";
  const checking = engine.state === "checking";
  return (
    <div className={`engine-badge ${active ? "is-active" : checking ? "is-checking" : "is-unavailable"}`}>
      <span className={active ? "engine-dot is-active" : checking ? "engine-dot is-checking" : "engine-dot"} />
      <div>
        <p><strong>{engine.name}</strong> {active ? "ACTIVO" : checking ? "COMPROBANDO" : "NO DISPONIBLE"}</p>
        <span>{engine.version ?? engine.detail ?? "Verificando motor local…"}</span>
      </div>
    </div>
  );
}

type UpdaterState = "idle" | "checking" | "available" | "downloading" | "installed" | "error";

interface UpdatePanelProps {
  state: UpdaterState;
  version?: string;
  message?: string;
  onCheck: () => void;
  onInstall: () => void;
}

function UpdatePanel({ state, version, message, onCheck, onInstall }: UpdatePanelProps) {
  const checking = state === "checking";
  const downloading = state === "downloading";
  const updateAvailable = state === "available";
  return (
    <section className="update-panel">
      <div>
        <p className="eyebrow">ACTUALIZACIONES</p>
        <span>Al abrir la app se buscan actualizaciones firmadas. La descarga e instalación sólo comienzan con tu confirmación.</span>
      </div>
      <div className="update-actions">
        <button className="secondary-button" disabled={checking || downloading} onClick={onCheck}><Search size={16} />{checking ? "BUSCANDO…" : "BUSCAR ACTUALIZACIÓN"}</button>
        {updateAvailable && <button className="primary-button small" disabled={downloading} onClick={onInstall}><Download size={16} />INSTALAR {version}</button>}
      </div>
      {message && <p className={`update-message ${state === "error" ? "error" : state === "available" ? "available" : ""}`}>{message}</p>}
    </section>
  );
}

interface UpdatePromptProps {
  version?: string;
  message?: string;
  onInstall: () => void;
  onDismiss: () => void;
}

function UpdatePrompt({ version, message, onInstall, onDismiss }: UpdatePromptProps) {
  return (
    <div className="update-prompt-backdrop" role="presentation">
      <section className="update-prompt" role="dialog" aria-modal="true" aria-labelledby="update-prompt-title">
        <div className="update-prompt-icon"><Download size={22} /></div>
        <div>
          <p className="eyebrow">ACTUALIZACIÓN DISPONIBLE</p>
          <h2 id="update-prompt-title">Hay una nueva versión lista para instalar</h2>
          <p>{version ? `Se encontró la versión ${version}.` : "Se encontró una nueva versión."} {message}</p>
          <div className="update-prompt-actions">
            <button className="secondary-button" onClick={onDismiss}>MÁS TARDE</button>
            <button className="primary-button small" onClick={onInstall}><Download size={16} />ACTUALIZAR AHORA</button>
          </div>
        </div>
      </section>
    </div>
  );
}

interface VideoCardProps {
  video: AnalyzedVideo;
  selection: DownloadSelection;
  alreadyQueued: boolean;
  onSelectionChange: (selection: Partial<DownloadSelection>) => void;
  onAdd: () => void;
  onRemove: () => void;
}

function VideoCard({ video, selection, alreadyQueued, onSelectionChange, onAdd, onRemove }: VideoCardProps) {
  const qualities = useMemo(() => [...video.qualities].sort((left, right) => right.height - left.height), [video.qualities]);
  const selected = qualities.find((option) => option.formatId === selection.selectedFormatId);
  const resolutionValue = selection.compatibilityMode ? "__compatibility__" : selected?.formatId ?? "";

  return (
    <article className="video-card">
      <div className="thumbnail-wrap">
        {video.thumbnail ? <img src={video.thumbnail} alt="" /> : <div className="thumbnail-fallback" />}
        <span className="duration-tag">{formatDuration(video.duration)}</span>
      </div>
      <div className="video-details">
        <h3>{video.title}</h3>
        <p>{video.channel ?? "Canal no disponible"}</p>
        <span className="ready-state"><CheckCircle2 size={13} /> LISTO PARA DESCARGAR</span>
      </div>
      <label className="select-field">
        <span>Resolución</span>
        <div className="select-wrap">
          <select
            aria-label={`Resolución para ${video.title}`}
            value={resolutionValue}
            onChange={(event) => {
              if (event.target.value === "__compatibility__") {
                onSelectionChange({ qualityHeight: null, selectedFormatId: null, selectedFormatHasAudio: null, compatibilityMode: true });
                return;
              }
              const quality = qualities.find((option) => option.formatId === event.target.value);
              if (quality) onSelectionChange({ qualityHeight: quality.height, selectedFormatId: quality.formatId, selectedFormatHasAudio: quality.formatHasAudio, compatibilityMode: false });
            }}
          >
            {qualities.length === 0 && <option value="">SIN CALIDAD ORIGINAL DISPONIBLE</option>}
            {qualities.map((option) => <option key={option.formatId} value={option.formatId}>{option.label}</option>)}
            <option disabled>──────── Compatibilidad ────────</option>
            <option value="__compatibility__">MEJOR CALIDAD COMPATIBLE (PUEDE SER INFERIOR)</option>
          </select>
          <ChevronDown size={15} />
        </div>
        <small>{selection.compatibilityMode ? "No es una calidad verificada: yt-dlp elegirá la mejor fuente que pueda descargar, aunque sea inferior." : selected ? `${selected.videoFormats.length} stream${selected.videoFormats.length === 1 ? "" : "s"} originales detectados` : "No hay una calidad original verificable"}</small>
      </label>
      <label className="select-field compact">
        <span>Formato</span>
        <div className="select-wrap">
          <select
            aria-label={`Formato para ${video.title}`}
            value={selection.container === "auto" ? "mp4" : selection.container}
            onChange={(event) => onSelectionChange({ container: event.target.value as DownloadContainer })}
          >
            <option value="mp4">MP4</option>
            <option value="mkv">MKV</option>
            <option value="webm">WEBM</option>
          </select>
          <ChevronDown size={15} />
        </div>
      </label>
      <button className="card-queue-button" disabled={alreadyQueued} onClick={onAdd}><Plus size={14} />{alreadyQueued ? "EN COLA" : "AGREGAR ESTE VIDEO"}</button>
      <button className="icon-button" aria-label={`Eliminar ${video.title}`} title="Quitar del análisis" onClick={onRemove}><Trash2 size={18} /></button>
    </article>
  );
}

interface QueueJobCardProps {
  job: DownloadJob;
  position: number;
  canStartIndividually: boolean;
  onStart: () => void;
  onCancel: () => void;
  onRetry: () => void;
  onOpenFile: () => void;
  onOpenFolder: () => void;
}

function QueueJobCard({ job, position, canStartIndividually, onStart, onCancel, onRetry, onOpenFile, onOpenFolder }: QueueJobCardProps) {
  const percent = job.progress.percent === null ? null : Math.max(0, Math.min(100, job.progress.percent));
  const isCompleted = job.status === "completed";
  const isFailed = job.status === "failed";
  const canCancel = isActiveStatus(job.status) || isWaitingStatus(job.status);

  return (
    <article className={`queue-job ${job.status}`}>
      <span className="queue-position">{String(position).padStart(2, "0")}</span>
      <div className="queue-thumb">
        {job.thumbnail ? <img src={job.thumbnail} alt="" /> : <div className="thumbnail-fallback" />}
      </div>
      <div className="queue-job-main">
        <div className="queue-job-title-row"><h3>{job.title}</h3><strong className={`job-status ${job.status}`}>{statusLabel(job.status)}</strong></div>
        <p>{jobQualityLabel(job)} · {job.container === "auto" ? "Auto" : job.container.toUpperCase()}</p>
        {(isActiveStatus(job.status) || isWaitingStatus(job.status)) && <>
          <div className="job-progress-track" aria-label={percent === null ? "Progreso no disponible" : `${Math.round(percent)}%`}><i style={{ width: `${percent ?? 0}%` }} /></div>
          <div className="job-metrics">
            <span>{job.progress.totalBytes === null ? formatBytes(job.progress.downloadedBytes) : `${formatBytes(job.progress.downloadedBytes)} / ${formatBytes(job.progress.totalBytes)}`}</span>
            {job.status === "downloading" && job.progress.speed !== null && <span>{formatSpeed(job.progress.speed)}</span>}
            {job.status === "downloading" && job.progress.eta !== null && <span>{formatEta(job.progress.eta)} restantes</span>}
            {percent !== null && <strong>{Math.round(percent)}%</strong>}
          </div>
        </>}
        {job.status === "processing" && <span className="job-detail">{job.message ?? "Fusionando video + audio"}</span>}
        {isFailed && <span className="job-error">{job.error ?? "La descarga no se pudo completar."}</span>}
        {job.status === "cancelled" && <span className="job-detail">{job.message ?? "Descarga cancelada."}</span>}
        {canStartIndividually && <button className="download-one-button" onClick={onStart}><Play size={14} fill="currentColor" />DESCARGAR ESTE VIDEO</button>}
        {isCompleted && <div className="completed-actions"><button className="inline-action" onClick={onOpenFile}><FileText size={14} />Abrir archivo</button><button className="inline-action" onClick={onOpenFolder}><FolderOpen size={14} />Abrir carpeta</button></div>}
        {isFailed && <button className="retry-button" onClick={onRetry}><RotateCcw size={14} />Reintentar</button>}
      </div>
      {canCancel && <button className="queue-cancel" onClick={onCancel} title="Cancelar descarga"><XCircle size={17} />Cancelar</button>}
    </article>
  );
}

interface HistoryEntryCardProps {
  entry: HistoryEntry;
  position: number;
  onOpenFile: () => void;
  onOpenFolder: () => void;
  onRemove: () => void;
}

function HistoryEntryCard({ entry, position, onOpenFile, onOpenFolder, onRemove }: HistoryEntryCardProps) {
  return (
    <article className="history-entry">
      <span className="queue-position">{String(position).padStart(2, "0")}</span>
      <div className="queue-thumb">
        {entry.thumbnail ? <img src={entry.thumbnail} alt="" /> : <div className="thumbnail-fallback" />}
      </div>
      <div className="queue-job-main">
        <div className="queue-job-title-row"><h3>{entry.title}</h3><strong className="job-status completed">COMPLETADO</strong></div>
        <p>{entry.resolution} · {entry.container === "auto" ? "Auto" : entry.container.toUpperCase()}</p>
        <span className="history-meta">{entry.channel ?? "Canal no disponible"} · {formatHistoryDate(entry.downloadedAt)}</span>
        <div className="completed-actions"><button className="inline-action" onClick={onOpenFile}><FileText size={14} />Abrir archivo</button><button className="inline-action" onClick={onOpenFolder}><FolderOpen size={14} />Abrir carpeta</button></div>
      </div>
      <button className="history-remove" onClick={onRemove} title="Quitar del historial; el archivo no se elimina"><Trash2 size={17} />Quitar</button>
    </article>
  );
}

export function App() {
  const {
    engines,
    videos,
    destination,
    isAnalyzing,
    selections,
    jobs,
    history,
    isQueuePaused,
    setEngines,
    setVideos,
    removeVideo,
    setDestination,
    setAnalyzing,
    setSelection,
    setQueueSnapshot,
    upsertJob,
    setHistory,
    removeHistory,
    applyDownloadEvent,
    setQueuePaused,
  } = useDownloadStore();
  const [urls, setUrls] = useState("");
  const [notice, setNotice] = useState<string>();
  const [error, setError] = useState<string>();
  const [analysisFailures, setAnalysisFailures] = useState<AnalysisFailure[]>([]);
  const [activeView, setActiveView] = useState("downloads");
  const [isQueueActionRunning, setQueueActionRunning] = useState(false);
  const [addingVideoIds, setAddingVideoIds] = useState<string[]>([]);
  const addingVideoIdsRef = useRef(new Set<string>());
  const pendingUpdateRef = useRef<Update | null>(null);
  const [updaterState, setUpdaterState] = useState<UpdaterState>("idle");
  const [updateVersion, setUpdateVersion] = useState<string>();
  const [updateMessage, setUpdateMessage] = useState<string>();
  const [isUpdatePromptOpen, setUpdatePromptOpen] = useState(false);
  const [appVersion, setAppVersion] = useState<string>();
  const [youtubeAccessMode, setYoutubeAccessMode] = useState<YoutubeAccessMode>(savedYoutubeAccessMode);

  useEffect(() => {
    void getVersion().then(setAppVersion).catch(() => undefined);
    void invoke<EngineInfo[]>("check_engines")
      .then(setEngines)
      .catch(() => setEngines([
        { name: "yt-dlp", state: "unavailable", detail: "Backend Tauri no iniciado" },
        { name: "ffmpeg", state: "unavailable", detail: "Backend Tauri no iniciado" },
        { name: "ffprobe", state: "unavailable", detail: "Backend Tauri no iniciado" },
        { name: "deno", state: "unavailable", detail: "Backend Tauri no iniciado" },
      ]));
    void invoke<string>("default_download_directory").then(setDestination).catch(() => setDestination(defaultDestination));
  }, [setDestination, setEngines]);

  useEffect(() => {
    let isMounted = true;
    const subscriptions: Array<Promise<UnlistenFn>> = [
      listen<DownloadEvent>("download://started", ({ payload }) => applyDownloadEvent(payload, "downloading")),
      listen<DownloadEvent>("download://progress", ({ payload }) => applyDownloadEvent(payload, "downloading")),
      listen<DownloadEvent>("download://processing", ({ payload }) => applyDownloadEvent(payload, "processing")),
      listen<DownloadEvent>("download://completed", ({ payload }) => {
        applyDownloadEvent(payload, "completed");
        if (isMounted) setNotice(payload.message ?? "La descarga se completó y fue validada.");
        void invoke<HistoryEntry[]>("get_history").then((entries) => {
          if (isMounted) setHistory(entries);
        }).catch(() => undefined);
      }),
      listen<DownloadEvent>("download://failed", ({ payload }) => {
        applyDownloadEvent(payload, "failed");
        if (isMounted) setError(payload.error ?? payload.message ?? "La descarga no se pudo completar.");
      }),
      listen<DownloadEvent>("download://cancelled", ({ payload }) => applyDownloadEvent(payload, "cancelled")),
      listen<QueueSnapshot>("queue://updated", ({ payload }) => setQueueSnapshot(payload)),
    ];
    void invoke<QueueSnapshot>("get_download_queue").then((snapshot) => {
      if (isMounted) setQueueSnapshot(snapshot);
    }).catch(() => undefined);
    void invoke<HistoryEntry[]>("get_history").then((entries) => {
      if (isMounted) setHistory(entries);
    }).catch(() => undefined);

    return () => {
      isMounted = false;
      void Promise.all(subscriptions).then((unlisten) => unlisten.forEach((stop) => stop()));
    };
  }, [applyDownloadEvent, setHistory, setQueueSnapshot]);

  useEffect(() => {
    void checkForUpdates({ silent: true, showPromptWhenAvailable: true });
    return () => {
      const update = pendingUpdateRef.current;
      pendingUpdateRef.current = null;
      if (update) void update.close().catch(() => undefined);
    };
  }, []);

  const engineReady = ["yt-dlp", "ffmpeg", "ffprobe", "deno"].every((name) => engines.some((engine) => engine.name === name && engine.state === "available"));
  const validLines = useMemo(() => urls.split(/\r?\n/).map((line) => line.trim()).filter(Boolean), [urls]);
  const uniqueUrls = useMemo(() => [...new Set(validLines)], [validLines]);
  const activeJobs = useMemo(() => jobs.filter((job) => isActiveStatus(job.status)), [jobs]);
  const waitingJobs = useMemo(() => jobs.filter((job) => isWaitingStatus(job.status)), [jobs]);
  const activeJob = activeJobs[0];
  const completedJobs = useMemo(() => jobs.filter((job) => job.status === "completed"), [jobs]);
  const queueProgress = useMemo(() => {
    if (jobs.length === 0) return 0;
    const completed = jobs.filter((job) => job.status === "completed").length;
    const activePercent = activeJob?.progress.percent ?? 0;
    return Math.round(((completed + activePercent / 100) / jobs.length) * 100);
  }, [activeJob?.progress.percent, jobs]);
  const hasDestination = Boolean(destination) && destination !== defaultDestination;
  const browserRecoveryFailure = analysisFailures.find((failure) => failure.requiresBrowserSession);

  function selectYoutubeAccessMode(mode: YoutubeAccessMode) {
    setYoutubeAccessMode(mode);
    try {
      if (mode === "public") window.localStorage.removeItem(youtubeAccessModeStorageKey);
      else window.localStorage.setItem(youtubeAccessModeStorageKey, mode);
    } catch {
      // The selection still applies for the current application session.
    }
  }

  async function refreshQueue() {
    try {
      setQueueSnapshot(await invoke<QueueSnapshot>("get_download_queue"));
    } catch {
      // The live queue event remains the source of truth when a refresh is unavailable.
    }
  }

  async function analyzeVideos(accessMode: YoutubeAccessMode = youtubeAccessMode) {
    setError(undefined);
    setNotice(undefined);
    setAnalysisFailures([]);
    if (uniqueUrls.length === 0) {
      setError("Pegá al menos una URL de YouTube, una por línea.");
      return;
    }
    if (!engineReady) {
      setError("No se puede analizar: yt-dlp, FFmpeg, ffprobe y Deno deben estar activos.");
      return;
    }
    setAnalyzing(true);
    try {
      const result = await invoke<AnalysisResult>("analyze_urls", {
        urls: uniqueUrls,
        browserSession: accessMode === "public" ? null : accessMode,
      });
      setVideos(result.videos);
      setAnalysisFailures(result.failures);
      if (result.videos.length > 0) setNotice(`${result.videos.length} video${result.videos.length === 1 ? "" : "s"} analizado${result.videos.length === 1 ? "" : "s"}. Elegí la calidad y agregalo a la cola.`);
      if (result.videos.length === 0 && result.failures.length > 0 && !result.failures.some((failure) => failure.requiresBrowserSession)) setError("No se pudo analizar ninguna de las URLs ingresadas.");
    } catch (reason) {
      setError(errorMessage(reason, "No se pudieron analizar las URLs. Verificá que los motores estén disponibles."));
    } finally {
      setAnalyzing(false);
    }
  }

  async function pasteLinks() {
    setError(undefined);
    try {
      const clipboardText = (await readText()).trim();
      if (!clipboardText) {
        setNotice("El portapapeles no contiene texto para pegar.");
        return;
      }
      setUrls((current) => current.trim() ? `${current.trimEnd()}\n${clipboardText}` : clipboardText);
      setNotice("Link pegado desde el portapapeles.");
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo leer el portapapeles."));
    }
  }

  function clearLinks() {
    setUrls("");
    setError(undefined);
    setNotice("Se limpiaron los links pegados.");
  }

  async function clearDownloadSession() {
    setError(undefined);
    if (activeJobs.length > 0 || waitingJobs.length > 0) {
      setError("Esperá a que terminen o cancelá las descargas pendientes antes de limpiar la sesión.");
      return;
    }
    setQueueActionRunning(true);
    try {
      await invoke<void>("clear_finished_downloads");
      setUrls("");
      setVideos([]);
      setAnalysisFailures([]);
      setNotice("Sesión limpia. Tus archivos descargados y el historial se conservaron.");
      await refreshQueue();
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo limpiar la sesión de descargas."));
    } finally {
      setQueueActionRunning(false);
    }
  }

  async function chooseDestination() {
    try {
      const folder = await open({ directory: true, multiple: false, title: "Elegí dónde guardar las descargas" });
      if (typeof folder === "string") setDestination(folder);
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo elegir la carpeta de destino."));
    }
  }

  async function refreshEngines() {
    setError(undefined);
    try {
      setEngines(await invoke<EngineInfo[]>("check_engines"));
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo comprobar el estado de los motores locales."));
    }
  }

  async function checkForUpdates({ silent = false, showPromptWhenAvailable = true }: { silent?: boolean; showPromptWhenAvailable?: boolean } = {}) {
    setUpdaterState("checking");
    setUpdateMessage(undefined);
    try {
      const update = await check({ timeout: 30_000 });
      if (pendingUpdateRef.current) {
        await pendingUpdateRef.current.close().catch(() => undefined);
        pendingUpdateRef.current = null;
      }
      if (!update) {
        setUpdateVersion(undefined);
        setUpdaterState("idle");
        setUpdatePromptOpen(false);
        if (!silent) setUpdateMessage("Ya tenés instalada la versión más reciente.");
        return;
      }
      pendingUpdateRef.current = update;
      setUpdateVersion(update.version);
      setUpdaterState("available");
      setUpdateMessage(update.body?.trim() || `Hay una nueva versión disponible: ${update.version}.`);
      setUpdatePromptOpen(showPromptWhenAvailable);
    } catch (reason) {
      setUpdateVersion(undefined);
      setUpdaterState("error");
      if (!silent) setUpdateMessage(errorMessage(reason, "No se pudo comprobar si hay actualizaciones."));
    }
  }

  async function installUpdate() {
    const update = pendingUpdateRef.current;
    if (!update) {
      await checkForUpdates();
      return;
    }
    setUpdatePromptOpen(false);
    setUpdaterState("downloading");
    setUpdateMessage("Descargando y verificando la actualización…");
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") setUpdateMessage("Descargando la actualización…");
        if (event.event === "Finished") setUpdateMessage("Verificación completada. Instalando la nueva versión…");
      });
      setUpdaterState("installed");
      setUpdateMessage("La actualización se instaló. Windows cerrará y abrirá la aplicación con la nueva versión.");
    } catch (reason) {
      setUpdaterState("error");
      setUpdateMessage(errorMessage(reason, "No se pudo instalar la actualización."));
    }
  }

  async function addVideoToQueue(video: AnalyzedVideo): Promise<boolean> {
    setError(undefined);
    if (!engineReady) {
      setError("No se puede agregar a la cola: yt-dlp, FFmpeg, ffprobe y Deno deben estar activos.");
      return false;
    }
    if (!hasDestination) {
      setError("Elegí una carpeta de destino antes de agregar una descarga.");
      return false;
    }
    const selection = selections[video.id] ?? defaultSelectionFor(video);
    const request: AddDownloadJobRequest = {
      videoId: video.id,
      url: video.url,
      title: video.title,
      thumbnail: video.thumbnail,
      channel: video.channel,
      qualityHeight: selection.qualityHeight,
      selectedFormatId: selection.selectedFormatId,
      selectedFormatHasAudio: selection.selectedFormatHasAudio,
      compatibilityMode: selection.compatibilityMode,
      browserSession: video.browserSession ?? null,
      usePotProvider: video.usePotProvider,
      container: selection.container,
      destination,
    };
    if (addingVideoIdsRef.current.has(video.id)) return false;
    addingVideoIdsRef.current.add(video.id);
    setAddingVideoIds((current) => [...current, video.id]);
    try {
      const job = await invoke<DownloadJob>("add_download_job", { request });
      upsertJob(job);
      setNotice(`Agregado a la cola: ${video.title}`);
      return true;
    } catch (reason) {
      setError(errorMessage(reason, `No se pudo agregar “${video.title}” a la cola.`));
      return false;
    } finally {
      addingVideoIdsRef.current.delete(video.id);
      setAddingVideoIds((current) => current.filter((id) => id !== video.id));
    }
  }

  async function addAllToQueue() {
    const queuedVideoIds = new Set(jobs.filter((job) => job.status !== "completed" && job.status !== "failed" && job.status !== "cancelled").map((job) => job.videoId));
    const pendingVideos = videos.filter((video) => !queuedVideoIds.has(video.id));
    if (pendingVideos.length === 0) {
      setNotice("Todos los videos analizados ya están en la cola.");
      return;
    }
    setQueueActionRunning(true);
    try {
      let added = 0;
      for (const video of pendingVideos) {
        if (await addVideoToQueue(video)) added += 1;
      }
      if (added > 0) setNotice(`${added} video${added === 1 ? "" : "s"} agregado${added === 1 ? "" : "s"} a la cola.`);
      await refreshQueue();
    } finally {
      setQueueActionRunning(false);
    }
  }

  async function startQueue() {
    setError(undefined);
    if (!engineReady) {
      setError("No se puede iniciar la cola: faltan motores requeridos.");
      return;
    }
    if (jobs.length === 0) {
      setError("Agregá al menos un video antes de iniciar la cola.");
      return;
    }
    setQueueActionRunning(true);
    try {
      await invoke<void>("start_download_queue");
      setQueuePaused(false);
      await refreshQueue();
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo iniciar la cola de descargas."));
    } finally {
      setQueueActionRunning(false);
    }
  }

  async function startSingleDownload(jobId: string) {
    setError(undefined);
    if (!engineReady) {
      setError("No se puede iniciar la descarga: faltan motores requeridos.");
      return;
    }
    if (activeJobs.length > 0) {
      setError("Esperá a que termine la descarga actual antes de iniciar otra.");
      return;
    }
    setQueueActionRunning(true);
    try {
      await invoke<void>("start_download_job", { jobId });
      setQueuePaused(false);
      await refreshQueue();
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo iniciar esta descarga."));
    } finally {
      setQueueActionRunning(false);
    }
  }

  async function toggleQueuePause() {
    setError(undefined);
    setQueueActionRunning(true);
    try {
      await invoke<void>(isQueuePaused ? "resume_download_queue" : "pause_download_queue");
      setQueuePaused(!isQueuePaused);
      await refreshQueue();
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo actualizar el estado de la cola."));
    } finally {
      setQueueActionRunning(false);
    }
  }

  async function cancelJob(jobId: string) {
    setError(undefined);
    try {
      await invoke<void>("cancel_download_job", { jobId });
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo cancelar la descarga."));
    }
  }

  async function cancelAll() {
    setError(undefined);
    setQueueActionRunning(true);
    try {
      await invoke<void>("cancel_all_downloads");
      await refreshQueue();
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo cancelar la cola."));
    } finally {
      setQueueActionRunning(false);
    }
  }

  async function retryJob(jobId: string) {
    setError(undefined);
    try {
      await invoke<void>("retry_download_job", { jobId });
      await refreshQueue();
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo reintentar la descarga."));
    }
  }

  async function openFile(jobId: string) {
    try {
      await invoke<void>("open_download_file", { jobId });
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo abrir el archivo descargado."));
    }
  }

  async function openFolder(jobId: string) {
    try {
      await invoke<void>("open_download_folder", { jobId });
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo abrir la carpeta de destino."));
    }
  }

  async function removeHistoryEntry(id: string) {
    try {
      await invoke<void>("remove_history_entry", { id });
      removeHistory(id);
      setNotice("La entrada se quitó del historial. El archivo descargado no fue eliminado.");
    } catch (reason) {
      setError(errorMessage(reason, "No se pudo quitar la entrada del historial."));
    }
  }

  return (
    <main className="app-shell">
      <header className="app-header" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region><span className="brand-logo" title="YT Downloader"><img src={logoImage} alt="Logo YT Downloader" /></span><strong><em>YT</em> DOWNLOAD</strong>{appVersion && <small>v{appVersion}</small>}</div>
        <div className="engines" data-tauri-drag-region>{engines.filter((engine) => engine.name === "yt-dlp" || engine.name === "ffmpeg").map((engine) => <EngineBadge engine={engine} key={engine.name} />)}</div>
      </header>

      <div className="workspace">
        <aside className="sidebar">
          <nav>
            {[{ id: "downloads", icon: Download, label: "Descargas" }, { id: "history", icon: History, label: "Historial" }, { id: "settings", icon: Settings, label: "Configuración" }, { id: "about", icon: Info, label: "Acerca de" }].map(({ id, icon: Icon, label }) => (
              <button key={id} className={activeView === id ? "nav-item active" : "nav-item"} onClick={() => setActiveView(id)}><Icon size={22} />{label}</button>
            ))}
          </nav>
          <section className="destination-box">
            <p>GUARDAR EN</p>
            <span title={destination}>{destination || defaultDestination}</span>
            <button onClick={() => void chooseDestination()}>Cambiar</button>
          </section>
        </aside>

        <section className="content-area">
          {activeView === "settings" && <UpdatePanel state={updaterState} version={updateVersion} message={updateMessage} onCheck={() => void checkForUpdates()} onInstall={() => void installUpdate()} />}
          {activeView === "downloads" ? <>
            <section className="url-panel panel">
              <p className="eyebrow">1. PEGÁ TUS LINKS DE YOUTUBE</p>
              <div className="url-entry">
                <div className="link-input-stage">
                  <div className="link-input-toolbar">
                    <div><button className="subtle-button" onClick={() => void pasteLinks()}><Clipboard size={15} />PEGAR</button><button className="icon-button clear-links-button" disabled={!urls.trim()} onClick={clearLinks} title="Limpiar todos los links" aria-label="Limpiar todos los links"><Trash2 size={17} /></button></div>
                  </div>
                  <textarea aria-label="Links de YouTube, uno por línea" value={urls} onChange={(event) => setUrls(event.target.value)} placeholder={"https://www.youtube.com/watch?v=…\nhttps://www.youtube.com/watch?v=…\n\nUn link por línea"} />
                </div>
                <button className="primary-button" disabled={isAnalyzing || !engineReady} onClick={() => void analyzeVideos()}><Search size={23} />{isAnalyzing ? "ANALIZANDO…" : "ANALIZAR VIDEOS"}</button>
              </div>
              {error && <p className="message error"><XCircle size={16} />{error}</p>}
              {notice && <p className="message success"><CheckCircle2 size={16} />{notice}</p>}
              {browserRecoveryFailure && <div className="youtube-recovery"><Info size={18} /><div><strong>SE NECESITA UNA SESIÓN DE YOUTUBE</strong><p>{browserRecoveryFailure.message}</p><ol className="youtube-recovery-steps"><li>Abrí <strong>Configuración</strong>.</li><li>En <strong>Acceso a YouTube</strong>, elegí Chrome o Edge donde ya usás YouTube.</li><li>Volvé a <strong>Descargas</strong> y presioná <strong>Analizar videos</strong>.</li></ol><button className="secondary-button" disabled={isAnalyzing} onClick={() => setActiveView("settings")}>IR A CONFIGURACIÓN</button></div></div>}
              {analysisFailures.filter((failure) => !failure.requiresBrowserSession).length > 0 && <div className="analysis-failures">{analysisFailures.filter((failure) => !failure.requiresBrowserSession).map((failure, index) => <p key={`${failure.url}-${index}`}><XCircle size={14} /><span>{failure.url}</span>{failure.message}</p>)}</div>}
              {!engineReady && <p className="engine-warning"><Info size={15} /> Para analizar y descargar, yt-dlp, FFmpeg, ffprobe y Deno deben estar activos.</p>}
            </section>

            <div className="workflow-stack">
              <section className="panel results-panel">
                <header className="section-header"><div><p className="eyebrow">2. VIDEOS ENCONTRADOS {videos.length > 0 ? `(${videos.length})` : ""}</p><span>Calidades detectadas directamente desde la fuente.</span></div><button className="secondary-button" disabled={videos.length === 0 || isQueueActionRunning || !engineReady || !hasDestination} onClick={() => void addAllToQueue()}><Plus size={17} />Agregar todos a la cola</button></header>
                {videos.length === 0 ? <div className="empty-state"><Download size={30} /><h2>PEGÁ TUS LINKS<br />PARA EMPEZAR</h2><p>Podés agregar uno o varios videos.<br />Cada link debe ir en una línea separada.</p></div> : <div className="video-list">{videos.map((video) => {
                  const alreadyQueued = jobs.some((job) => job.videoId === video.id && job.status !== "completed" && job.status !== "failed" && job.status !== "cancelled");
                  return <VideoCard key={video.id} video={video} selection={selections[video.id] ?? defaultSelectionFor(video)} alreadyQueued={alreadyQueued || addingVideoIds.includes(video.id)} onSelectionChange={(selection) => setSelection(video.id, selection)} onAdd={() => void addVideoToQueue(video)} onRemove={() => removeVideo(video.id)} />;
                })}</div>}
              </section>

              <section className="panel queue-panel">
                <header className="section-header"><div><p className="eyebrow">3. COLA DE DESCARGAS</p><span>{activeJobs.length} descargando · {waitingJobs.length} esperando{isQueuePaused ? " · pausada" : ""}</span></div><div className="queue-actions"><button className="subtle-button" disabled={jobs.length === 0 || isQueueActionRunning} onClick={() => void toggleQueuePause()}>{isQueuePaused ? <Play size={14} fill="currentColor" /> : <Pause size={14} fill="currentColor" />}{isQueuePaused ? "Reanudar cola" : "Pausar cola"}</button><button className="danger-button" disabled={jobs.length === 0 || isQueueActionRunning} onClick={() => void cancelAll()}><XCircle size={15} />Cancelar todo</button></div></header>
                {jobs.length === 0 ? <div className="queue-empty"><Clock3 size={27} /><strong>LA COLA ESTÁ VACÍA</strong><span>Analizá videos y agregalos a la cola<br />para iniciar las descargas.</span></div> : <div className="queue-list">{jobs.map((job, index) => <QueueJobCard key={job.jobId} job={job} position={index + 1} canStartIndividually={isWaitingStatus(job.status) && activeJobs.length === 0 && !isQueueActionRunning && engineReady} onStart={() => void startSingleDownload(job.jobId)} onCancel={() => void cancelJob(job.jobId)} onRetry={() => void retryJob(job.jobId)} onOpenFile={() => void openFile(job.jobId)} onOpenFolder={() => void openFolder(job.jobId)} />)}</div>}
                <footer className="queue-footer"><span>{isQueuePaused ? "La cola está pausada" : activeJob ? "Procesando una descarga" : waitingJobs.length > 0 ? "Elegí un video o descargá toda la cola" : "Sin descargas pendientes"}</span><button className="primary-button small" disabled={waitingJobs.length === 0 || activeJobs.length > 0 || isQueueActionRunning || !engineReady} onClick={() => void startQueue()}><Play size={16} fill="currentColor" />DESCARGAR TODA LA COLA</button></footer>
              </section>
            </div>

            <section className="download-stage">
              <header className="download-stage-heading">
                <div><p className="eyebrow">4. DESCARGA Y PROGRESO</p><span>El estado de la descarga aparece aquí mientras se procesa.</span></div>
                <button className="subtle-button clear-session-button" disabled={jobs.length === 0 || activeJobs.length > 0 || waitingJobs.length > 0 || isQueueActionRunning} onClick={() => void clearDownloadSession()}><Trash2 size={15} />LIMPIAR SESIÓN</button>
              </header>
              <div className="bottom-grid">
              <article className="panel activity-panel"><p className="eyebrow">ACTIVIDAD ACTUAL</p>{activeJob ? <div className="activity-live"><LoaderCircle size={20} /><div><strong>{activeJob.status === "processing" ? "Procesando video…" : "Descargando video…"}</strong><span>{activeJob.message ?? activeJob.title}</span><small>{activeJob.status === "downloading" && activeJob.progress.speed !== null ? `Velocidad ${formatSpeed(activeJob.progress.speed)}` : activeJob.status === "processing" ? "Fusionando video + audio" : "Esperando datos de yt-dlp"}</small></div></div> : <div className="activity-idle"><LoaderCircle size={20} /><span>El motor estará listo para procesar cuando agregues videos a la cola.</span></div>}</article>
              <article className="panel progress-panel"><p className="eyebrow">PROGRESO GENERAL</p><div className="progress-idle"><span>{queueProgress}%</span><div><i style={{ width: `${queueProgress}%` }} /></div><small>{jobs.length === 0 ? "No hay descargas activas" : `${completedJobs.length} de ${jobs.length} trabajo${jobs.length === 1 ? "" : "s"} completado${completedJobs.length === 1 ? "" : "s"}`}</small></div></article>
              </div>
            </section>
          </> : activeView === "settings" ? <section className="panel diagnostics-panel"><header><div><h1>CONFIGURACIÓN</h1><p>DIAGNÓSTICO DEL MOTOR</p></div><button className="secondary-button" onClick={() => void refreshEngines()}><LoaderCircle size={16} />VOLVER A COMPROBAR</button></header><section className="youtube-access-setting"><div><strong>ACCESO A YOUTUBE</strong><p>Usá <strong>Solo acceso público</strong> normalmente. La app primero intenta una verificación local automática; elegí Chrome o Edge sólo si en Descargas aparece un aviso indicándotelo. La app no guarda, copia ni muestra contraseñas o cookies.</p></div><label className="select-field"><span>Sesión para YouTube</span><div className="select-wrap"><select aria-label="Sesión local para YouTube" value={youtubeAccessMode} onChange={(event) => selectYoutubeAccessMode(event.target.value as YoutubeAccessMode)}><option value="public">SOLO ACCESO PÚBLICO</option><option value="chrome">USAR SESIÓN LOCAL DE CHROME</option><option value="edge">USAR SESIÓN LOCAL DE EDGE</option></select><ChevronDown size={15} /></div><small>Elegí el navegador donde ya usás YouTube; después volvé a Descargas y analizá el enlace de nuevo.</small></label></section><div className="diagnostic-list">{engines.map((engine) => <article key={engine.name}><span className={engine.state === "available" ? "engine-dot is-active" : engine.state === "checking" ? "engine-dot is-checking" : "engine-dot is-unavailable"} /><div><strong>{engine.name}</strong><p>Estado: {engine.state === "available" ? "Activo" : engine.state === "checking" ? "Comprobando" : "No disponible"}</p><p>Versión: {engine.version ?? "—"}</p><p className="diagnostic-path">Ruta: {engine.path ?? engine.detail ?? "—"}</p></div></article>)}</div></section> : activeView === "history" ? <section className="panel history-panel"><header className="section-header"><div><p className="eyebrow">HISTORIAL RECIENTE</p><span>Descargas persistidas localmente.</span></div></header>{history.length === 0 ? <div className="queue-empty"><History size={27} /><strong>SIN DESCARGAS COMPLETADAS</strong><span>Cuando una descarga termine correctamente,<br />aparecerá aquí incluso después de reiniciar.</span></div> : <div className="queue-list">{history.map((entry, index) => <HistoryEntryCard key={entry.id} entry={entry} position={index + 1} onOpenFile={() => void openFile(entry.id)} onOpenFolder={() => void openFolder(entry.id)} onRemove={() => void removeHistoryEntry(entry.id)} />)}</div>}</section> : <section className="panel secondary-view"><h1>ACERCA DE</h1><p>YT Download usa yt-dlp, FFmpeg, ffprobe y Deno empaquetados localmente para analizar y procesar tus descargas.</p><FolderOpen size={28} /></section>}
        </section>
      </div>
      {isUpdatePromptOpen && updaterState === "available" && <UpdatePrompt version={updateVersion} message={updateMessage} onInstall={() => void installUpdate()} onDismiss={() => setUpdatePromptOpen(false)} />}
    </main>
  );
}
