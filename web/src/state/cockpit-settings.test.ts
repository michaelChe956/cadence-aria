import { beforeEach, describe, expect, it } from "vitest";
import { readCockpitSettings } from "./cockpit-settings";

const STORAGE_KEY = "aria.cockpit.settings.v1";

describe("cockpit settings storage", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("returns the contract defaults when storage is empty", () => {
    expect(readCockpitSettings(window.localStorage)).toEqual({
      soundEnabled: false,
      systemNotificationsEnabled: true,
      titleEmojiEnabled: true,
      watchLimit: 8,
      gateOpenEscalationMs: 600_000,
      escalationRepeatMs: 300_000,
      escalationBudgetThreshold: 1,
      escalationBudgetRepeats: [1, 2, 3],
      stopPoints: ["human_gate", "stopped", "hard_error"],
    });
  });

  it("normalizes malformed persisted values without changing valid siblings", () => {
    window.localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        soundEnabled: true,
        watchLimit: 0,
        gateOpenEscalationMs: -1,
        stopPoints: ["human_gate", "invalid"],
      }),
    );

    expect(readCockpitSettings(window.localStorage)).toMatchObject({
      soundEnabled: true,
      watchLimit: 8,
      gateOpenEscalationMs: 600_000,
      stopPoints: ["human_gate"],
    });
  });

});
