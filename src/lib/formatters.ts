export function formatDuration(seconds?: number): string {
  if (!seconds || !Number.isFinite(seconds)) return "—";
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = Math.floor(seconds % 60);
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, "0")}:${rest.toString().padStart(2, "0")}`
    : `${minutes}:${rest.toString().padStart(2, "0")}`;
}

export function qualityLabel(height: number): string {
  if (height >= 2160) return `${height}p · 4K`;
  if (height >= 1440) return `${height}p · 2K`;
  if (height >= 1080) return `${height}p · Full HD`;
  if (height >= 720) return `${height}p · HD`;
  return `${height}p`;
}
