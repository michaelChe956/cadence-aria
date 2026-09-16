import { describe, expect, it } from "vitest";
import {
  CHAT_COCKPIT_STORAGE_KEY,
  readChatCockpitMode,
  writeChatCockpitMode,
} from "./chat-cockpit-mode";

describe("chat cockpit mode switch", () => {
  it("defaults according to the workspace type and generation phase when nothing is stored", () => {
    expect(readChatCockpitMode("work_item_plan")).toBe("legacy");
    expect(readChatCockpitMode("work_item_plan", true)).toBe("cockpit");
    expect(readChatCockpitMode("story")).toBe("legacy");
    expect(readChatCockpitMode("design")).toBe("legacy");
    expect(readChatCockpitMode(null)).toBe("legacy");
  });

  it("honours the cockpit value", () => {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, "cockpit");

    expect(readChatCockpitMode("story")).toBe("cockpit");
  });

  it("honours the legacy value", () => {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, "legacy");

    expect(readChatCockpitMode("work_item_plan", true)).toBe("legacy");
  });

  it("falls back to the type-aware default for unknown values", () => {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, "cockpit-v2");

    expect(readChatCockpitMode("work_item_plan", true)).toBe("cockpit");
  });

  it("persists an explicit mode", () => {
    writeChatCockpitMode("cockpit");
    expect(window.localStorage.getItem(CHAT_COCKPIT_STORAGE_KEY)).toBe("cockpit");

    writeChatCockpitMode("legacy");
    expect(window.localStorage.getItem(CHAT_COCKPIT_STORAGE_KEY)).toBe("legacy");
  });

  it("uses the safe legacy default when storage throws", () => {
    const original = window.localStorage.getItem;
    Object.defineProperty(window.localStorage, "getItem", {
      configurable: true,
      value: () => {
        throw new Error("storage disabled");
      },
    });

    expect(readChatCockpitMode("work_item_plan")).toBe("legacy");

    Object.defineProperty(window.localStorage, "getItem", {
      configurable: true,
      value: original,
    });
  });
});
