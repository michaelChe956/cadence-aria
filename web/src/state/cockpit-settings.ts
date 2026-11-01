export const COCKPIT_SETTINGS_STORAGE_KEY = "aria.cockpit.settings.v1";

export type CockpitStopPoint = "human_gate" | "stopped" | "hard_error";

export interface CockpitSettings {
  soundEnabled: boolean;
  systemNotificationsEnabled: boolean;
  titleEmojiEnabled: boolean;
  watchLimit: number;
  observerRefreshIntervalMs: number;
  gateOpenEscalationMs: number;
  escalationRepeatMs: number;
  escalationBudgetThreshold: number;
  escalationBudgetRepeats: readonly number[];
  stopPoints: readonly CockpitStopPoint[];
}

export const DEFAULT_COCKPIT_SETTINGS: Readonly<CockpitSettings> = {
  soundEnabled: false,
  systemNotificationsEnabled: true,
  titleEmojiEnabled: true,
  watchLimit: 8,
  observerRefreshIntervalMs: 15_000,
  gateOpenEscalationMs: 600_000,
  escalationRepeatMs: 300_000,
  escalationBudgetThreshold: 1,
  escalationBudgetRepeats: [1, 2, 3],
  stopPoints: ["human_gate", "stopped", "hard_error"],
};

const OBSERVER_REFRESH_INTERVAL_OPTIONS = [5_000, 15_000, 30_000, 60_000] as const;
const GATE_OPEN_ESCALATION_OPTIONS = [60_000, 300_000, 600_000, 900_000] as const;
const ESCALATION_REPEAT_OPTIONS = [60_000, 300_000, 600_000] as const;


export function readCockpitSettings(storage: Storage = window.localStorage): CockpitSettings {
  try {
    const stored = storage.getItem(COCKPIT_SETTINGS_STORAGE_KEY);
    return normalizeCockpitSettings(stored === null ? null : JSON.parse(stored));
  } catch {
    return normalizeCockpitSettings(null);
  }
}

export function writeCockpitSettings(
  settings: CockpitSettings,
  storage: Storage = window.localStorage,
): void {
  try {
    storage.setItem(
      COCKPIT_SETTINGS_STORAGE_KEY,
      JSON.stringify(normalizeCockpitSettings(settings)),
    );
  } catch {
    // localStorage 不可用时静默降级，不影响当前页面状态。
  }
}

type PersistedCockpitSettings = Partial<
  Record<keyof CockpitSettings, unknown>
>;

function normalizeCockpitSettings(value: unknown): CockpitSettings {
  const candidate: PersistedCockpitSettings =
    typeof value === "object" && value !== null && !Array.isArray(value)
      ? (value as PersistedCockpitSettings)
      : {};
  const defaults = DEFAULT_COCKPIT_SETTINGS;

  return {
    soundEnabled:
      typeof candidate.soundEnabled === "boolean"
        ? candidate.soundEnabled
        : defaults.soundEnabled,
    systemNotificationsEnabled:
      typeof candidate.systemNotificationsEnabled === "boolean"
        ? candidate.systemNotificationsEnabled
        : defaults.systemNotificationsEnabled,
    titleEmojiEnabled:
      typeof candidate.titleEmojiEnabled === "boolean"
        ? candidate.titleEmojiEnabled
        : defaults.titleEmojiEnabled,
    watchLimit:
      typeof candidate.watchLimit === "number" &&
      Number.isInteger(candidate.watchLimit) &&
      candidate.watchLimit >= 1 &&
      candidate.watchLimit <= 32
        ? candidate.watchLimit
        : defaults.watchLimit,
    observerRefreshIntervalMs: selectDurationOption(
      candidate.observerRefreshIntervalMs,
      OBSERVER_REFRESH_INTERVAL_OPTIONS,
      defaults.observerRefreshIntervalMs,
    ),
    gateOpenEscalationMs: selectDurationOption(
      candidate.gateOpenEscalationMs,
      GATE_OPEN_ESCALATION_OPTIONS,
      defaults.gateOpenEscalationMs,
    ),
    escalationRepeatMs: selectDurationOption(
      candidate.escalationRepeatMs,
      ESCALATION_REPEAT_OPTIONS,
      defaults.escalationRepeatMs,
    ),
    escalationBudgetThreshold:
      typeof candidate.escalationBudgetThreshold === "number" &&
      Number.isInteger(candidate.escalationBudgetThreshold) &&
      candidate.escalationBudgetThreshold > 0
        ? candidate.escalationBudgetThreshold
        : defaults.escalationBudgetThreshold,
    escalationBudgetRepeats:
      Array.isArray(candidate.escalationBudgetRepeats) &&
      candidate.escalationBudgetRepeats.every(
        (item) =>
          typeof item === "number" && Number.isInteger(item) && item > 0,
      )
        ? candidate.escalationBudgetRepeats
        : defaults.escalationBudgetRepeats,
    stopPoints: Array.isArray(candidate.stopPoints)
      ? candidate.stopPoints.filter(
          (item): item is CockpitStopPoint =>
            item === "human_gate" ||
            item === "stopped" ||
            item === "hard_error",
        )
      : defaults.stopPoints,
  };
}

function selectDurationOption(
  value: unknown,
  options: readonly number[],
  fallback: number,
): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value <= 0) {
    return fallback;
  }

  return options.reduce((closest, option) =>
    Math.abs(option - value) < Math.abs(closest - value) ? option : closest,
  );
}
