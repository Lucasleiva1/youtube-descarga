import { create } from "zustand";
import type {
  AnalyzedVideo,
  DownloadEvent,
  DownloadJob,
  DownloadSelection,
  DownloadStatus,
  EngineInfo,
  HistoryEntry,
  QueueSnapshot,
} from "../types/download";

const emptyProgress = {
  percent: null,
  speed: null,
  eta: null,
  downloadedBytes: 0,
  totalBytes: null,
};

function defaultSelection(video?: AnalyzedVideo): DownloadSelection {
  const highestQuality = video?.qualities.reduce<number | null>((highest, quality) => highest === null || quality.height > highest ? quality.height : highest, null) ?? null;
  return { qualityHeight: highestQuality, container: "mp4" };
}

function normalizeJob(job: DownloadJob): DownloadJob {
  return {
    ...job,
    progress: { ...emptyProgress, ...job.progress },
  };
}

interface DownloadStore {
  engines: EngineInfo[];
  videos: AnalyzedVideo[];
  destination: string;
  isAnalyzing: boolean;
  selections: Record<string, DownloadSelection>;
  jobs: DownloadJob[];
  history: HistoryEntry[];
  isQueuePaused: boolean;
  setEngines: (engines: EngineInfo[]) => void;
  setVideos: (videos: AnalyzedVideo[]) => void;
  removeVideo: (videoId: string) => void;
  setDestination: (destination: string) => void;
  setAnalyzing: (isAnalyzing: boolean) => void;
  setSelection: (videoId: string, selection: Partial<DownloadSelection>) => void;
  setQueueSnapshot: (snapshot: QueueSnapshot) => void;
  upsertJob: (job: DownloadJob) => void;
  setHistory: (history: HistoryEntry[]) => void;
  removeHistory: (id: string) => void;
  applyDownloadEvent: (event: DownloadEvent, status: DownloadStatus) => void;
  setQueuePaused: (isQueuePaused: boolean) => void;
}

export const useDownloadStore = create<DownloadStore>((set) => ({
  engines: [
    { name: "yt-dlp", state: "checking" },
    { name: "ffmpeg", state: "checking" },
    { name: "ffprobe", state: "checking" },
    { name: "deno", state: "checking" },
  ],
  videos: [],
  destination: "",
  isAnalyzing: false,
  selections: {},
  jobs: [],
  history: [],
  isQueuePaused: false,
  setEngines: (engines) => set({ engines }),
  setVideos: (videos) => set((state) => {
    const selections = Object.fromEntries(videos.map((video) => {
      const current = state.selections[video.id];
      const stillAvailable = current?.qualityHeight === null
        || video.qualities.some((quality) => quality.height === current?.qualityHeight);
      return [video.id, current && stillAvailable ? { ...current, container: current.container === "auto" ? "mp4" : current.container } : defaultSelection(video)];
    }));
    return { videos, selections };
  }),
  removeVideo: (videoId) => set((state) => {
    const { [videoId]: _removed, ...selections } = state.selections;
    return { videos: state.videos.filter((video) => video.id !== videoId), selections };
  }),
  setDestination: (destination) => set({ destination }),
  setAnalyzing: (isAnalyzing) => set({ isAnalyzing }),
  setSelection: (videoId, selection) => set((state) => ({
    selections: {
      ...state.selections,
      [videoId]: { ...(state.selections[videoId] ?? defaultSelection()), ...selection },
    },
  })),
  setQueueSnapshot: (snapshot) => set({
    jobs: snapshot.jobs.map(normalizeJob),
    isQueuePaused: snapshot.isPaused ?? snapshot.paused ?? false,
  }),
  upsertJob: (job) => set((state) => {
    const normalized = normalizeJob(job);
    const index = state.jobs.findIndex((item) => item.jobId === normalized.jobId);
    if (index === -1) return { jobs: [...state.jobs, normalized] };
    const jobs = [...state.jobs];
    jobs[index] = normalized;
    return { jobs };
  }),
  setHistory: (history) => set({ history }),
  removeHistory: (id) => set((state) => ({ history: state.history.filter((entry) => entry.id !== id) })),
  applyDownloadEvent: (event, status) => set((state) => {
    const index = state.jobs.findIndex((job) => job.jobId === event.jobId);
    if (index === -1) {
      if (!event.job) return state;
      return {
        jobs: [...state.jobs, normalizeJob({
          ...event.job,
          status,
          message: event.message ?? event.job.message,
          error: event.error ?? event.job.error,
          progress: { ...event.job.progress, ...event.progress },
        })],
      };
    }

    const existing = state.jobs[index];
    const incoming = event.job;
    const jobs = [...state.jobs];
    jobs[index] = normalizeJob({
      ...existing,
      ...incoming,
      status,
      message: event.message ?? incoming?.message ?? existing.message,
      error: event.error ?? incoming?.error ?? existing.error,
      progress: {
        ...existing.progress,
        ...(incoming?.progress ?? {}),
        ...(event.progress ?? {}),
      },
    });
    return { jobs };
  }),
  setQueuePaused: (isQueuePaused) => set({ isQueuePaused }),
}));
