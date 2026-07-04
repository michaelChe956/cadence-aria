export function formatDurationMs(durationMs: number): string | null {
  if (!Number.isFinite(durationMs) || durationMs < 0) {
    return null;
  }

  const totalSeconds = Math.round(durationMs / 1000);
  if (totalSeconds < 60) {
    return `${totalSeconds}秒`;
  }

  const totalMinutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (totalMinutes < 60) {
    return `${totalMinutes}分${seconds.toString().padStart(2, "0")}秒`;
  }

  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return `${hours}小时${minutes.toString().padStart(2, "0")}分`;
}

export function elapsedDurationText(
  startedAt?: string | null,
  completedAt?: string | null,
  durationMs?: number | null,
): string | null {
  if (typeof durationMs === "number") {
    return formatDurationMs(durationMs);
  }
  if (!startedAt || !completedAt) {
    return null;
  }

  const started = Date.parse(startedAt);
  const completed = Date.parse(completedAt);
  if (!Number.isFinite(started) || !Number.isFinite(completed)) {
    return null;
  }

  return formatDurationMs(completed - started);
}
