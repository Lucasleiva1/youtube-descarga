export type EngineState = "checking" | "available" | "unavailable";

export interface EngineInfo {
  name: "yt-dlp" | "ffmpeg" | "ffprobe" | "deno";
  state: EngineState;
  version?: string;
  detail?: string;
  path?: string;
}

export interface VideoFormat {
  id: string;
  extension: string;
  height?: number;
  width?: number;
  fps?: number;
  videoCodec?: string;
  audioCodec?: string;
  bitrate?: number;
  filesize?: number;
  filesizeApprox?: number;
  hasVideo: boolean;
  hasAudio: boolean;
}

export interface QualityOption {
  height: number;
  label: string;
  formatId: string;
  formatHasAudio: boolean;
  videoFormats: VideoFormat[];
}

export type BrowserSession = "chrome" | "edge";

export interface AnalyzedVideo {
  id: string;
  url: string;
  title: string;
  channel?: string;
  duration?: number;
  thumbnail?: string;
  /** Browser session explicitly chosen when this source was analyzed. */
  browserSession?: BrowserSession | null;
  qualities: QualityOption[];
  formats: VideoFormat[];
}

export interface AnalysisFailure {
  url: string;
  message: string;
}

export interface AnalysisResult {
  videos: AnalyzedVideo[];
  failures: AnalysisFailure[];
}

export type DownloadContainer = "auto" | "mp4" | "mkv" | "webm";

export type DownloadStatus =
  | "pending"
  | "analyzing"
  | "ready"
  | "queued"
  | "downloading"
  | "processing"
  | "completed"
  | "failed"
  | "cancelled";

/** The selection made in the analysis card before a job is created. */
export interface DownloadSelection {
  /** null means "MEJOR CALIDAD" and is resolved by yt-dlp using real formats. */
  qualityHeight: number | null;
  /** Exact yt-dlp stream selected during analysis; never a guessed resolution. */
  selectedFormatId: string | null;
  selectedFormatHasAudio: boolean | null;
  /** Explicit opt-in only: yt-dlp chooses its best compatible source stream. */
  compatibilityMode: boolean;
  container: DownloadContainer;
}

/** Request sent to Rust when the user adds a real download to the queue. */
export interface AddDownloadJobRequest {
  videoId: string;
  url: string;
  title: string;
  thumbnail?: string | null;
  channel?: string | null;
  qualityHeight: number | null;
  selectedFormatId: string | null;
  selectedFormatHasAudio: boolean | null;
  compatibilityMode: boolean;
  browserSession?: BrowserSession | null;
  container: DownloadContainer;
  destination: string;
}

/** Values parsed from yt-dlp's live progress output. They are never simulated. */
export interface DownloadProgress {
  percent: number | null;
  speed: number | null;
  eta: number | null;
  downloadedBytes: number;
  totalBytes: number | null;
}

export interface DownloadVerification {
  width?: number;
  height?: number;
  duration?: number;
  videoCodec?: string;
  audioCodec?: string;
}

export interface DownloadJob extends AddDownloadJobRequest {
  jobId: string;
  status: DownloadStatus;
  progress: DownloadProgress;
  message?: string;
  error?: string;
  filePath?: string;
  createdAt?: string;
  completedAt?: string;
  verification?: DownloadVerification;
}

/**
 * Structured payload emitted by the download manager. A job is included on
 * lifecycle changes and progress is included only when yt-dlp supplies it.
 */
export interface DownloadEvent {
  jobId: string;
  job?: DownloadJob;
  progress?: Partial<DownloadProgress>;
  message?: string;
  error?: string;
}

export interface QueueSnapshot {
  jobs: DownloadJob[];
  /** Current Rust contract. */
  isPaused?: boolean;
  /** Kept optional so a snapshot from an older development build still renders. */
  paused?: boolean;
}

/** A completed download persisted by the local SQLite history database. */
export interface HistoryEntry {
  id: string;
  videoId: string;
  url: string;
  title: string;
  thumbnail?: string | null;
  channel?: string | null;
  resolution: string;
  container: string;
  filePath: string;
  /** Unix epoch in seconds, returned by SQLite. */
  downloadedAt: number;
}
