import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { ONBOARDING_SEEN_KEY, useOnboarding, type UseOnboardingResult } from "./useOnboarding";

function stubStorageUnavailable() {
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
  return () => {
    if (original) {
      Object.defineProperty(window, "localStorage", original);
    }
  };
}

describe("useOnboarding", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("首次进入（无已读记录）自动打开且从第一个启用步骤开始", () => {
    const { result } = renderHook(() => useOnboarding());
    expect(result.current.open).toBe(true);
    expect(result.current.index).toBe(0);
    expect(result.current.total).toBe(13);
    expect(result.current.current?.id).toBe("create-project");
  });

  it("已读记录存在时不再自动展示", () => {
    window.localStorage.setItem(ONBOARDING_SEEN_KEY, "1");
    const { result } = renderHook(() => useOnboarding());
    expect(result.current.open).toBe(false);
  });

  it("跳过写入已读并关闭", () => {
    const { result } = renderHook(() => useOnboarding());
    act(() => result.current.skip());
    expect(window.localStorage.getItem(ONBOARDING_SEEN_KEY)).not.toBeNull();
    expect(result.current.open).toBe(false);
  });

  it("完成/关闭同样写入已读并关闭", () => {
    const { result } = renderHook(() => useOnboarding());
    act(() => result.current.close());
    expect(window.localStorage.getItem(ONBOARDING_SEEN_KEY)).not.toBeNull();
    expect(result.current.open).toBe(false);
  });

  it("帮助入口重开：从第一步开始且不清除已读记录", () => {
    window.localStorage.setItem(ONBOARDING_SEEN_KEY, "1");
    const { result } = renderHook(() => useOnboarding());
    expect(result.current.open).toBe(false);

    act(() => result.current.next());
    act(() => result.current.reopen());
    expect(result.current.open).toBe(true);
    expect(result.current.index).toBe(0);
    expect(window.localStorage.getItem(ONBOARDING_SEEN_KEY)).toBe("1");
  });

  it("上下步在启用步骤边界内收敛", () => {
    const { result } = renderHook(() => useOnboarding());
    act(() => result.current.prev());
    expect(result.current.index).toBe(0);

    for (let i = 0; i < 20; i += 1) {
      act(() => result.current.next());
    }
    expect(result.current.index).toBe(12);
    expect(result.current.current?.id).toBe("coding-progress-complete");
  });

  it("未完成时卸载不写入已读（不误记）", () => {
    const { unmount } = renderHook(() => useOnboarding());
    unmount();
    expect(window.localStorage.getItem(ONBOARDING_SEEN_KEY)).toBeNull();
  });

  it("localStorage 不可用时静默降级为不弹、不抛错", () => {
    const restore = stubStorageUnavailable();
    let result: { current: UseOnboardingResult } | null = null;
    let didThrow = false;
    try {
      result = renderHook(() => useOnboarding()).result;
    } catch {
      didThrow = true;
    }
    expect(didThrow).toBe(false);
    if (!result) {
      throw new Error("renderHook did not return a result");
    }
    expect(result.current.open).toBe(false);
    // 存储不可用仍可经帮助入口手动打开。
    act(() => result.current.reopen());
    expect(result.current.open).toBe(true);
    restore();
  });
});
