import { create } from "zustand";
import type { AnalyzedVideo, EngineInfo } from "../types/download";

interface DownloadStore {
  engines: EngineInfo[];
  videos: AnalyzedVideo[];
  destination: string;
  isAnalyzing: boolean;
  setEngines: (engines: EngineInfo[]) => void;
  setVideos: (videos: AnalyzedVideo[]) => void;
  setDestination: (destination: string) => void;
  setAnalyzing: (isAnalyzing: boolean) => void;
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
  setEngines: (engines) => set({ engines }),
  setVideos: (videos) => set({ videos }),
  setDestination: (destination) => set({ destination }),
  setAnalyzing: (isAnalyzing) => set({ isAnalyzing }),
}));
