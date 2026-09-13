import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitSettings } from "../../state/cockpit-settings";
import type { CockpitInboxItem } from "../../state/workspace-cockpit-projection";
import { CockpitEscalation, evaluateEscalations } from "./CockpitEscalation";

const now = Date.UTC(2026, 8, 14, 12, 0, 0);

class AudioContextMock {
  static instances: AudioContextMock[] = [];

  state: AudioContextState = "suspended";
  currentTime = 0;
  resume = vi.fn(async () => {
    this.state = "running";
  });
  close = vi.fn(async () => {
    this.state = "closed";
  });
  createOscillator = vi.fn(() => ({
    frequency: { value: 0 },
    connect: vi.fn(),
    start: vi.fn(),
    stop: vi.fn(),
    addEventListener: vi.fn(),
  }));
  createGain = vi.fn(() => ({
    gain: {
      setValueAtTime: vi.fn(),
      exponentialRampToValueAtTime: vi.fn(),
    },
    connect: vi.fn(),
  }));

  constructor() {
    AudioContextMock.instances.push(this);
  }
}

afterEach(() => {
  AudioContextMock.instances = [];
  vi.unstubAllGlobals();
});

function settings(overrides: Partial<CockpitSettings> = {}): CockpitSettings {
  return {
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
    ...overrides,
  };
}

function gateItem(
  id: string,
  overrides: Partial<NonNullable<CockpitInboxItem["gate"]>> = {},
): CockpitInboxItem {
  return {
    id,
    kind: "gate",
    severity: 1,
    title: "门禁等待",
    summary: "等待人工确认",
    triage: false,
    source: "gate",
    createdAt: new Date(now).toISOString(),
    gate: {
      key: id,
      turn_id: null,
      stage: "human_confirm",
      flow_kind: "single_candidate",
      status: "open",
      trigger: null,
      remaining_budget: null,
      findings: [],
      resumable: false,
      triage: false,
      closed: null,
      closure_stage: null,
      opened_at: new Date(now).toISOString(),
      turn: null,
      ...overrides,
    },
    inlineError: null,
  };
}

describe("evaluateEscalations", () => {
  it("turns remaining budget at the configured threshold orange and escalates at configured repeats", () => {
    const item = gateItem("gate-a", { remaining_budget: 1 });
    const initial = evaluateEscalations([item], settings(), now, new Map());

    expect(initial).toContainEqual({
      key: "budget:gate-a",
      kind: "budget",
      level: 1,
      itemId: "gate-a",
    });

    const repeated = evaluateEscalations(
      [item],
      settings({ escalationBudgetRepeats: [1, 3, 5] }),
      now,
      new Map([["budget:gate-a", now - 4 * 300_000]]),
    );

    expect(repeated).toContainEqual(
      expect.objectContaining({ kind: "budget", level: 2 }),
    );

    const finalRepeat = evaluateEscalations(
      [item],
      settings({ escalationBudgetRepeats: [1, 3, 5] }),
      now,
      new Map([["budget:gate-a", now - 5 * 300_000]]),
    );

    expect(finalRepeat).toContainEqual(
      expect.objectContaining({ kind: "budget", level: 3 }),
    );
  });

  it("escalates a frontend-timed gate at ten minutes without claiming engine timeout", () => {
    const item = gateItem("gate-a", {
      opened_at: new Date(now - 600_000).toISOString(),
    });

    expect(evaluateEscalations([item], settings(), now, new Map())).toContainEqual(
      expect.objectContaining({ kind: "gate_age", level: 1 }),
    );

    render(<CockpitEscalation items={[item]} settings={settings()} now={now} />);

    expect(screen.getByText("前端计时提醒阈值已达到")).toBeVisible();
    expect(screen.queryByText(/引擎超时/)).toBeNull();
  });

  it("honors a configured gate-age threshold", () => {
    const item = gateItem("gate-a", {
      opened_at: new Date(now - 5_000).toISOString(),
    });

    expect(
      evaluateEscalations(
        [item],
        settings({ gateOpenEscalationMs: 5_000 }),
        now,
        new Map(),
      ),
    ).toContainEqual(expect.objectContaining({ kind: "gate_age" }));
  });
  it("renders the budget trigger with the warning semantic token", () => {
    render(
      <CockpitEscalation
        items={[gateItem("gate-a", { remaining_budget: 1 })]}
        settings={settings()}
        now={now}
      />,
    );

    expect(screen.getByTestId("cockpit-escalation-budget")).toHaveClass(
      "border-[var(--aria-warning)]",
      "bg-[var(--aria-warning-soft)]",
    );
  });

  it("renders the configured final repeat at level three", () => {
    const playSound = vi.fn();
    const item = gateItem("gate-a", { remaining_budget: 1 });
    const view = render(
      <CockpitEscalation
        items={[item]}
        settings={settings({ soundEnabled: true, escalationBudgetRepeats: [1, 2, 3] })}
        now={now}
        playSound={playSound}
      />,
    );

    view.rerender(
      <CockpitEscalation
        items={[item]}
        settings={settings({ soundEnabled: true, escalationBudgetRepeats: [1, 2, 3] })}
        now={now + 3 * 300_000}
        playSound={playSound}
      />,
    );

    expect(screen.getByTestId("cockpit-escalation-budget")).toHaveAttribute(
      "data-escalation-level",
      "3",
    );
  });
});

describe("CockpitEscalation", () => {
  it("limits multiple gates to one audible escalation per repeat interval while retaining every visual item", () => {
    const playSound = vi.fn();
    const items = [
      gateItem("gate-a", { opened_at: new Date(now - 600_000).toISOString() }),
      gateItem("gate-b", { opened_at: new Date(now - 600_000).toISOString() }),
    ];

    render(
      <CockpitEscalation
        items={items}
        settings={settings({ soundEnabled: true })}
        now={now}
        playSound={playSound}
      />,
    );
    expect(playSound).toHaveBeenCalledTimes(1);

    expect(screen.getAllByTestId("cockpit-escalation-gate_age")).toHaveLength(2);
  });

  it("uses one repeat interval for newly due gates", () => {
    const playSound = vi.fn();
    const firstGate = gateItem("gate-a", { remaining_budget: 1 });
    const secondGate = gateItem("gate-b", { remaining_budget: 1 });
    const view = render(
      <CockpitEscalation
        items={[firstGate]}
        settings={settings({ soundEnabled: true })}
        now={now}
        playSound={playSound}
      />,
    );

    view.rerender(
      <CockpitEscalation
        items={[firstGate, secondGate]}
        settings={settings({ soundEnabled: true })}
        now={now + 1}
        playSound={playSound}
      />,
    );

    expect(playSound).toHaveBeenCalledTimes(1);
  });

  it("allows the next audible escalation after the configured repeat interval", () => {
    const playSound = vi.fn();
    const firstGate = gateItem("gate-a", { remaining_budget: 1 });
    const secondGate = gateItem("gate-b", { remaining_budget: 1 });
    const view = render(
      <CockpitEscalation
        items={[firstGate]}
        settings={settings({ soundEnabled: true })}
        now={now}
        playSound={playSound}
      />,
    );

    view.rerender(
      <CockpitEscalation
        items={[firstGate, secondGate]}
        settings={settings({ soundEnabled: true })}
        now={now + 300_000}
        playSound={playSound}
      />,
    );

    expect(playSound).toHaveBeenCalledTimes(2);
  });

  it("keeps sound disabled by default", () => {
    const playSound = vi.fn();

    render(
      <CockpitEscalation
        items={[gateItem("gate-a", { remaining_budget: 1 })]}
        settings={settings()}
        now={now}
        playSound={playSound}
      />,
    );

    expect(playSound).not.toHaveBeenCalled();
  });

  it("does not replay a low-budget audible escalation before the configured interval", () => {
    const playSound = vi.fn();
    const item = gateItem("gate-a", { remaining_budget: 1 });
    const view = render(
      <CockpitEscalation
        items={[item]}
        settings={settings({ soundEnabled: true })}
        now={now}
        playSound={playSound}
      />,
    );

    view.rerender(
      <CockpitEscalation
        items={[item]}
        settings={settings({ soundEnabled: true })}
        now={now + 299_999}
        playSound={playSound}
      />,
    );

    expect(playSound).toHaveBeenCalledTimes(1);
  });

  it("resumes and reuses one audio context across due intervals", async () => {
    vi.stubGlobal("AudioContext", AudioContextMock);
    const item = gateItem("gate-a", { remaining_budget: 1 });
    const view = render(
      <CockpitEscalation
        items={[item]}
        settings={settings({ soundEnabled: true })}
        now={now}
      />,
    );

    await act(async () => {});
    view.rerender(
      <CockpitEscalation
        items={[item]}
        settings={settings({ soundEnabled: true })}
        now={now + 300_000}
      />,
    );
    await act(async () => {});

    expect(AudioContextMock.instances).toHaveLength(1);
    expect(AudioContextMock.instances[0]?.resume).toHaveBeenCalledTimes(1);
  });

  it("contains audio context construction failures", () => {
    vi.stubGlobal("AudioContext", () => {
      throw new Error("audio unavailable");
    });

    expect(() =>
      render(
        <CockpitEscalation
          items={[gateItem("gate-a", { remaining_budget: 1 })]}
          settings={settings({ soundEnabled: true })}
          now={now}
        />,
      ),
    ).not.toThrow();
  });
});
