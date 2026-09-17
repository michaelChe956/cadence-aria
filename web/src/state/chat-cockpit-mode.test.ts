import { describe, expect, it } from "vitest";
import {
  CHAT_COCKPIT_STORAGE_KEY,
  readChatCockpitMode,
  writeChatCockpitMode,
} from "./chat-cockpit-mode";

describe("chat cockpit mode switch", () => {
  it("defaults known workspace types to cockpit when nothing is stored", () => {
    expect(readChatCockpitMode("work_item_plan")).toBe("cockpit");
    expect(readChatCockpitMode("story")).toBe("cockpit");
    expect(readChatCockpitMode("design")).toBe("cockpit");
    expect(readChatCockpitMode(null)).toBe("legacy");
  });

  it("honours the cockpit value", () => {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, "cockpit");

    expect(readChatCockpitMode("story")).toBe("cockpit");
  });

  it("honours the legacy value", () => {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, "legacy");

    expect(readChatCockpitMode("work_item_plan")).toBe("legacy");
  });

  it("falls back to the type-aware default for unrecognized stored values", () => {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, "cockpit-v2");

    expect(readChatCockpitMode("work_item_plan")).toBe("cockpit");
    expect(readChatCockpitMode("story")).toBe("cockpit");
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

    expect(readChatCockpitMode("work_item_plan")).toBe("cockpit");

    Object.defineProperty(window.localStorage, "getItem", {
      configurable: true,
      value: original,
    });
  });
});
