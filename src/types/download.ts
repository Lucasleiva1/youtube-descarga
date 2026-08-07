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
  videoFormats: VideoFormat[];
}

export interface AnalyzedVideo {
  id: string;
  url: string;
  title: string;
  channel?: string;
  duration?: number;
  thumbnail?: string;
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
