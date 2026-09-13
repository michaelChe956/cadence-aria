import { useEffect, useMemo, useRef, useState } from "react";
import type { CockpitSettings } from "../../state/cockpit-settings";
import type { CockpitInboxItem } from "../../state/workspace-cockpit-projection";

export interface CockpitEscalation {
  key: string;
  kind: "budget" | "gate_age";
  level: 1 | 2 | 3;
  itemId: string;
}

const ONE_SECOND_MS = 1_000;

type PlaySound = () => void;

function escalationLevel(
  settings: CockpitSettings,
  now: number,
  lastNotifiedAt: number | undefined,
): 1 | 2 | 3 | null {
  if (lastNotifiedAt === undefined) {
    return 1;
  }

  const elapsed = now - lastNotifiedAt;
  if (elapsed < settings.escalationRepeatMs) {
    return null;
  }

  const repeatCount = Math.floor(elapsed / settings.escalationRepeatMs);
  const configuredLevel =
    settings.escalationBudgetRepeats.filter((repeat) => repeat <= repeatCount).length;
  return configuredLevel >= 3 ? 3 : configuredLevel === 2 ? 2 : 1;
}

function openGate(item: CockpitInboxItem): NonNullable<CockpitInboxItem["gate"]> | null {
  if (item.kind !== "gate" || item.gate === null || item.gate.closed !== null) {
    return null;
  }
  return item.gate;
}

export function evaluateEscalations(
  items: readonly CockpitInboxItem[],
  settings: CockpitSettings,
  now: number,
  lastNotifiedAt: ReadonlyMap<string, number>,
): readonly CockpitEscalation[] {
  const escalations: CockpitEscalation[] = [];

  for (const item of items) {
    const gate = openGate(item);
    if (gate === null) {
      continue;
    }

    if (
      gate.remaining_budget !== null &&
      gate.remaining_budget <= settings.escalationBudgetThreshold
    ) {
      const key = `budget:${item.id}`;
      const level = escalationLevel(settings, now, lastNotifiedAt.get(key));
      if (level !== null) {
        escalations.push({ key, kind: "budget", level, itemId: item.id });
      }
    }

    const openedAt = Date.parse(gate.opened_at);
    if (
      !Number.isNaN(openedAt) &&
      now - openedAt >= settings.gateOpenEscalationMs
    ) {
      const key = `gate_age:${item.id}`;
      const level = escalationLevel(settings, now, lastNotifiedAt.get(key));
      if (level !== null) {
        escalations.push({ key, kind: "gate_age", level, itemId: item.id });
      }
    }
  }

  return escalations;
}

function activeEscalationKeys(
  items: readonly CockpitInboxItem[],
  settings: CockpitSettings,
  now: number,
): ReadonlySet<string> {
  return new Set(
    evaluateEscalations(items, settings, now, new Map()).map((escalation) => escalation.key),
  );
}

function defaultPlaySound(): void {
  if (typeof window === "undefined" || !("AudioContext" in window)) {
    return;
  }

  const context = new AudioContext();
  const oscillator = context.createOscillator();
  const gain = context.createGain();
  const startAt = context.currentTime;
  oscillator.frequency.value = 880;
  gain.gain.setValueAtTime(0.08, startAt);
  gain.gain.exponentialRampToValueAtTime(0.001, startAt + 0.12);
  oscillator.connect(gain);
  gain.connect(context.destination);
  oscillator.start(startAt);
  oscillator.stop(startAt + 0.12);
  oscillator.addEventListener("ended", () => {
    void context.close();
  });
}

function useEscalationNow(now: number | undefined): number {
  const [tickedNow, setTickedNow] = useState(() => Date.now());

  useEffect(() => {
    if (now !== undefined) {
      return;
    }
    const timer = window.setInterval(() => setTickedNow(Date.now()), ONE_SECOND_MS);
    return () => window.clearInterval(timer);
  }, [now]);

  return now ?? tickedNow;
}

function sameEscalations(
  left: ReadonlyMap<string, CockpitEscalation>,
  right: ReadonlyMap<string, CockpitEscalation>,
): boolean {
  if (left.size !== right.size) {
    return false;
  }
  for (const [key, escalation] of left) {
    const candidate = right.get(key);
    if (
      candidate === undefined ||
      candidate.kind !== escalation.kind ||
      candidate.level !== escalation.level ||
      candidate.itemId !== escalation.itemId
    ) {
      return false;
    }
  }
  return true;
}

function escalationCopy(item: CockpitInboxItem, escalation: CockpitEscalation): string {
  if (escalation.kind === "budget") {
    return `剩余修复轮次 ${item.gate?.remaining_budget ?? 0}，请优先处理`;
  }
  return "前端计时提醒阈值已达到";
}

export function CockpitEscalation({
  items,
  settings,
  now,
  playSound = defaultPlaySound,
}: {
  items: readonly CockpitInboxItem[];
  settings: CockpitSettings;
  now?: number;
  playSound?: PlaySound;
}) {
  const currentNow = useEscalationNow(now);
  const firstEscalatedAtRef = useRef(new Map<string, number>());
  const lastAudibleAtRef = useRef<number | null>(null);
  const [visibleEscalations, setVisibleEscalations] = useState<ReadonlyMap<
    string,
    CockpitEscalation
  >>(() => new Map());
  const itemById = useMemo(
    () => new Map(items.map((item) => [item.id, item])),
    [items],
  );

  useEffect(() => {
    const activeKeys = activeEscalationKeys(items, settings, currentNow);
    const firstEscalatedAt = firstEscalatedAtRef.current;
    for (const key of Array.from(firstEscalatedAt.keys())) {
      if (!activeKeys.has(key)) {
        firstEscalatedAt.delete(key);
      }
    }
    if (activeKeys.size === 0) {
      lastAudibleAtRef.current = null;
    }

    const dueEscalations = evaluateEscalations(
      items,
      settings,
      currentNow,
      firstEscalatedAt,
    );
    for (const escalation of dueEscalations) {
      if (!firstEscalatedAt.has(escalation.key)) {
        firstEscalatedAt.set(escalation.key, currentNow);
      }
    }

    setVisibleEscalations((previous) => {
      const next = new Map<string, CockpitEscalation>();
      for (const [key, escalation] of previous) {
        if (activeKeys.has(key)) {
          next.set(key, escalation);
        }
      }
      for (const escalation of dueEscalations) {
        next.set(escalation.key, escalation);
      }
      return sameEscalations(previous, next) ? previous : next;
    });

    if (
      settings.soundEnabled &&
      dueEscalations.length > 0 &&
      (lastAudibleAtRef.current === null ||
        currentNow - lastAudibleAtRef.current >= settings.escalationRepeatMs)
    ) {
      playSound();
      lastAudibleAtRef.current = currentNow;
    }
  }, [currentNow, items, playSound, settings]);

  if (visibleEscalations.size === 0) {
    return null;
  }

  return (
    <aside
      aria-label="门禁升级提醒"
      className="fixed bottom-4 right-4 z-[100] flex max-w-sm flex-col gap-2"
    >
      {Array.from(visibleEscalations.values()).map((escalation) => {
        const item = itemById.get(escalation.itemId);
        if (!item) {
          return null;
        }
        return (
          <div
            key={escalation.key}
            data-escalation-level={escalation.level}
            data-testid={`cockpit-escalation-${escalation.kind}`}
            className="rounded-lg border-2 border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-sm text-[var(--aria-ink)] shadow-md motion-safe:transition-colors motion-safe:duration-200"
          >
            <p className="font-semibold">{item.title}</p>
            <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
              {escalationCopy(item, escalation)}
            </p>
          </div>
        );
      })}
    </aside>
  );
}
