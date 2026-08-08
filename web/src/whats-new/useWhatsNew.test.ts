import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { CURRENT_VERSION } from "./changelog";
import { useWhatsNew } from "./useWhatsNew";

const SEEN_KEY = "aria-whats-new-seen";

describe("useWhatsNew", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("未读当前版本时 open 为 true 并提供对应 entry", () => {
    const { result } = renderHook(() => useWhatsNew());
    expect(result.current.open).toBe(true);
    expect(result.current.entry?.version).toBe(CURRENT_VERSION);
  });

  it("已读当前版本时 open 为 false", () => {
    window.localStorage.setItem(SEEN_KEY, CURRENT_VERSION);
    const { result } = renderHook(() => useWhatsNew());
    expect(result.current.open).toBe(false);
  });

  it("close 后写入已读版本号并关闭", () => {
    const { result } = renderHook(() => useWhatsNew());
    act(() => result.current.close());
    expect(window.localStorage.getItem(SEEN_KEY)).toBe(CURRENT_VERSION);
    expect(result.current.open).toBe(false);
  });

  it("localStorage 不可用时静默降级为不弹、不抛错", () => {
    const original = Object.getOwnPropertyDescriptor(window, "localStorage");
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: {
        getItem: () => {
          throw new Error("unavailable");
        },
        setItem: () => {
          throw new Error("unavailable");
        },
      },
    });
    let result: { current: ReturnType<typeof useWhatsNew> } | null = null;
    let didThrow = false;
    try {
      result = renderHook(() => useWhatsNew()).result;
    } catch {
      didThrow = true;
    }
    expect(didThrow).toBe(false);
    if (!result) {
      throw new Error("renderHook did not return a result");
    }
    expect(result.current.open).toBe(false);
    expect(result.current.entry?.version).toBe(CURRENT_VERSION);

    if (original) {
      Object.defineProperty(window, "localStorage", original);
    }
  });
});
