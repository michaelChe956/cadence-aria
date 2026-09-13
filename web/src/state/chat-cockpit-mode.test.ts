import { describe, expect, it } from "vitest";
import {
  CHAT_COCKPIT_STORAGE_KEY,
  readChatCockpitMode,
  writeChatCockpitMode,
} from "./chat-cockpit-mode";

describe("chat cockpit mode switch", () => {
  it("defaults to the legacy form when nothing is stored", () => {
    expect(readChatCockpitMode()).toBe("legacy");
  });

  it("honours the cockpit value", () => {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, "cockpit");

    expect(readChatCockpitMode()).toBe("cockpit");
  });

  it("falls back to legacy for unknown values", () => {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, "cockpit-v2");

    expect(readChatCockpitMode()).toBe("legacy");
  });

  it("persists an explicit mode", () => {
    writeChatCockpitMode("cockpit");
    expect(window.localStorage.getItem(CHAT_COCKPIT_STORAGE_KEY)).toBe("cockpit");

    writeChatCockpitMode("legacy");
    expect(window.localStorage.getItem(CHAT_COCKPIT_STORAGE_KEY)).toBe("legacy");
  });

  it("ignores a storage that throws", () => {
    const original = window.localStorage.getItem;
    Object.defineProperty(window.localStorage, "getItem", {
      configurable: true,
      value: () => {
        throw new Error("storage disabled");
      },
    });

    expect(readChatCockpitMode()).toBe("legacy");

    Object.defineProperty(window.localStorage, "getItem", {
      configurable: true,
      value: original,
    });
  });
});
