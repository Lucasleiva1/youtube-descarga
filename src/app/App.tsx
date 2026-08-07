import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import {
  CheckCircle2,
  ChevronDown,
  Clock3,
  Download,
  FolderOpen,
  History,
  Info,
  LoaderCircle,
  Play,
  Plus,
  Search,
  Settings,
  Trash2,
  XCircle,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { formatDuration } from "../lib/formatters";
import { useDownloadStore } from "../stores/downloadStore";
import type { AnalysisResult, AnalyzedVideo, EngineInfo } from "../types/download";

const defaultDestination = "Elegí una carpeta de destino";

function EngineBadge({ engine }: { engine: EngineInfo }) {
  const active = engine.state === "available";
  const checking = engine.state === "checking";
  return (
    <div className="engine-badge">
      <span className={active ? "engine-dot is-active" : checking ? "engine-dot is-checking" : "engine-dot"} />
      <div>
        <p><strong>{engine.name}</strong> {active ? "ACTIVO" : checking ? "COMPROBANDO" : "NO DISPONIBLE"}</p>
        <span>{engine.version ?? engine.detail ?? "Verificando motor local…"}</span>
      </div>
    </div>
  );
}

function VideoCard({ video }: { video: AnalyzedVideo }) {
  const [quality, setQuality] = useState(video.qualities[video.qualities.length - 1]?.height ?? 0);
  const selected = video.qualities.find((option) => option.height === quality);
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
          <select value={quality} onChange={(event) => setQuality(Number(event.target.value))}>
            {video.qualities.map((option) => <option key={option.height} value={option.height}>{option.label}</option>)}
          </select>
          <ChevronDown size={15} />
        </div>
        {selected && <small>{selected.videoFormats.length} stream{selected.videoFormats.length === 1 ? "" : "s"} detectados</small>}
      </label>
      <label className="select-field compact">
        <span>Formato</span>
        <div className="select-wrap"><select defaultValue="auto"><option value="auto">Auto</option><option value="mp4">MP4</option><option value="mkv">MKV</option><option value="webm">WEBM</option></select><ChevronDown size={15} /></div>
      </label>
      <button className="icon-button" aria-label={`Eliminar ${video.title}`} title="Quitar del análisis"><Trash2 size={18} /></button>
    </article>
  );
}

export function App() {
  const { engines, videos, destination, isAnalyzing, setEngines, setVideos, setDestination, setAnalyzing } = useDownloadStore();
  const [urls, setUrls] = useState("");
  const [notice, setNotice] = useState<string>();
  const [error, setError] = useState<string>();
  const [activeView, setActiveView] = useState("downloads");

  useEffect(() => {
    void invoke<EngineInfo[]>("check_engines")
      .then(setEngines)
      .catch(() => setEngines(engines.map((engine) => ({ ...engine, state: "unavailable", detail: "Backend Tauri no iniciado" }))));
    void invoke<string>("default_download_directory").then(setDestination).catch(() => setDestination(defaultDestination));
  // Initial engine verification belongs to the desktop shell lifecycle.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const engineReady = engines.some((engine) => engine.name === "yt-dlp" && engine.state === "available");
  const validLines = useMemo(() => urls.split(/\r?\n/).map((line) => line.trim()).filter(Boolean), [urls]);

  async function analyzeVideos() {
    setError(undefined);
    setNotice(undefined);
    if (validLines.length === 0) {
      setError("Pegá al menos una URL de YouTube, una por línea.");
      return;
    }
    setAnalyzing(true);
    try {
      const result = await invoke<AnalysisResult>("analyze_urls", { urls: validLines });
      setVideos(result.videos);
      if (result.failures.length > 0) setError(result.failures.map((failure) => `${failure.url}: ${failure.message}`).join(" · "));
      if (result.videos.length > 0) setNotice(`${result.videos.length} video${result.videos.length === 1 ? "" : "s"} analizado${result.videos.length === 1 ? "" : "s"}. Elegí la calidad para cada uno.`);
    } catch (reason) {
      setError(typeof reason === "string" ? reason : "No se pudieron analizar las URLs. Verificá que yt-dlp esté disponible.");
    } finally {
      setAnalyzing(false);
    }
  }

  async function chooseDestination() {
    const folder = await open({ directory: true, multiple: false, title: "Elegí dónde guardar las descargas" });
    if (typeof folder === "string") setDestination(folder);
  }

  async function refreshEngines() {
    setError(undefined);
    try {
      setEngines(await invoke<EngineInfo[]>("check_engines"));
    } catch {
      setError("No se pudo comprobar el estado de los motores locales.");
    }
  }

  return (
    <main className="app-shell">
      <header className="app-header" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region><span className="play-mark"><Play size={16} fill="currentColor" /></span><strong><em>YT</em> DOWNLOAD</strong><small>v0.1.0</small></div>
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
          {activeView === "downloads" ? <>
            <section className="url-panel panel">
              <p className="eyebrow">1. PEGÁ TUS LINKS DE YOUTUBE</p>
              <div className="url-entry">
                <textarea value={urls} onChange={(event) => setUrls(event.target.value)} placeholder={"https://www.youtube.com/watch?v=…\nhttps://www.youtube.com/watch?v=…\n\nUn link por línea"} />
                <button className="primary-button" disabled={isAnalyzing} onClick={() => void analyzeVideos()}><Search size={23} />{isAnalyzing ? "ANALIZANDO…" : "ANALIZAR VIDEOS"}</button>
              </div>
              {error && <p className="message error"><XCircle size={16} />{error}</p>}
              {notice && <p className="message success"><CheckCircle2 size={16} />{notice}</p>}
              {!engineReady && <p className="engine-warning"><Info size={15} /> yt-dlp no está disponible aún. La interfaz está lista, pero necesitás instalar o empaquetar el motor para analizar videos.</p>}
            </section>

            <div className="split-grid">
              <section className="panel results-panel">
                <header className="section-header"><div><p className="eyebrow">2. VIDEOS ENCONTRADOS {videos.length > 0 ? `(${videos.length})` : ""}</p><span>Calidades detectadas directamente desde la fuente.</span></div><button className="secondary-button" disabled={videos.length === 0}><Plus size={17} />Agregar todos a la cola</button></header>
                {videos.length === 0 ? <div className="empty-state"><Download size={30} /><h2>PEGÁ TUS LINKS<br />PARA EMPEZAR</h2><p>Podés agregar uno o varios videos.<br />Cada link debe ir en una línea separada.</p></div> : <div className="video-list">{videos.map((video) => <VideoCard video={video} key={video.id} />)}</div>}
              </section>

              <section className="panel queue-panel">
                <header className="section-header"><div><p className="eyebrow">3. COLA DE DESCARGAS</p><span>0 descargando · 0 esperando</span></div><div className="queue-actions"><button className="subtle-button" disabled>Pausar cola</button><button className="danger-button" disabled>Cancelar todo</button></div></header>
                <div className="queue-empty"><Clock3 size={27} /><strong>LA COLA ESTÁ VACÍA</strong><span>Analizá videos y agregalos a la cola<br />para iniciar las descargas.</span></div>
                <footer className="queue-footer"><span>Esperando descargas</span><button className="primary-button small" disabled><Play size={16} fill="currentColor" />INICIAR COLA</button></footer>
              </section>
            </div>

            <section className="bottom-grid">
              <article className="panel activity-panel"><p className="eyebrow">ACTIVIDAD ACTUAL</p><div className="activity-idle"><LoaderCircle size={20} /><span>El motor estará listo para procesar cuando agregues videos a la cola.</span></div></article>
              <article className="panel progress-panel"><p className="eyebrow">PROGRESO GENERAL</p><div className="progress-idle"><span>0%</span><div><i /></div><small>No hay descargas activas</small></div></article>
            </section>
          </> : activeView === "settings" ? <section className="panel diagnostics-panel"><header><div><h1>CONFIGURACIÓN</h1><p>DIAGNÓSTICO DEL MOTOR</p></div><button className="secondary-button" onClick={() => void refreshEngines()}><LoaderCircle size={16} />VOLVER A COMPROBAR</button></header><div className="diagnostic-list">{engines.map((engine) => <article key={engine.name}><span className={engine.state === "available" ? "engine-dot is-active" : "engine-dot"} /><div><strong>{engine.name}</strong><p>Estado: {engine.state === "available" ? "Activo" : engine.state === "checking" ? "Comprobando" : "No disponible"}</p><p>Versión: {engine.version ?? "—"}</p><p className="diagnostic-path">Ruta: {engine.path ?? engine.detail ?? "—"}</p></div></article>)}</div></section> : <section className="panel secondary-view"><h1>{activeView === "history" ? "HISTORIAL" : "ACERCA DE"}</h1><p>Esta sección se conectará a la persistencia local en la siguiente entrega del flujo de descargas.</p><FolderOpen size={28} /></section>}
        </section>
      </div>
    </main>
  );
}
