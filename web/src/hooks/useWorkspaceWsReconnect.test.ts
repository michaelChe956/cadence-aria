import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceWsReconnect } from "./useWorkspaceWsReconnect";

describe("useWorkspaceWsReconnect", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    setDocumentHidden(false);
  });

  afterEach(() => {
    vi.useRealTimers();
    setDocumentHidden(false);
  });

  it("starts with the initial delay after an abnormal close", () => {
    const onReconnect = vi.fn();

    renderHook(() =>
      useWorkspaceWsReconnect({
        enabled: true,
        onReconnect,
        closeCode: 1006,
      }),
    );

    act(() => {
      vi.advanceTimersByTime(999);
    });
    expect(onReconnect).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(onReconnect).toHaveBeenCalledTimes(1);
  });

  it("does not reconnect after a normal close", () => {
    const onReconnect = vi.fn();

    renderHook(() =>
      useWorkspaceWsReconnect({
        enabled: true,
        onReconnect,
        closeCode: 1000,
      }),
    );

    act(() => {
      vi.advanceTimersByTime(5000);
    });

    expect(onReconnect).not.toHaveBeenCalled();
  });

  it("reconnects at the hidden base delay after an abnormal close while hidden", () => {
    const onReconnect = vi.fn();

    setDocumentHidden(true);
    renderHook(() =>
      useWorkspaceWsReconnect({
        enabled: true,
        onReconnect,
        closeCode: 1006,
      }),
    );

    act(() => {
      vi.advanceTimersByTime(59999);
    });
    expect(onReconnect).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(onReconnect).toHaveBeenCalledTimes(1);
  });

  it("keeps a pending reconnect when backgrounded and slows to the hidden base delay", () => {
    const onReconnect = vi.fn();

    renderHook(() =>
      useWorkspaceWsReconnect({
        enabled: true,
        onReconnect,
        closeCode: 1006,
      }),
    );

    act(() => {
      setDocumentHidden(true);
      document.dispatchEvent(new Event("visibilitychange"));
    });

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(onReconnect).toHaveBeenCalledTimes(1);

    act(() => {
      vi.advanceTimersByTime(59999);
    });
    expect(onReconnect).toHaveBeenCalledTimes(1);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(onReconnect).toHaveBeenCalledTimes(2);
  });

  it("keeps the visible backoff delay across enabled toggles", () => {
    const onReconnect = vi.fn();
    const { rerender } = renderHook(
      ({ enabled }) =>
        useWorkspaceWsReconnect({
          enabled,
          onReconnect,
          closeCode: 1006,
        }),
      { initialProps: { enabled: true } },
    );

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(onReconnect).toHaveBeenCalledTimes(1);

    // 模拟真实接线:重连尝试令 status 翻转,enabled false→true 各自独立提交,
    // 使 close effect 重跑
    act(() => {
      rerender({ enabled: false });
    });
    act(() => {
      rerender({ enabled: true });
    });
    act(() => {
      vi.advanceTimersByTime(1001);
    });
    expect(onReconnect).toHaveBeenCalledTimes(1);

    act(() => {
      vi.advanceTimersByTime(1599);
    });
    expect(onReconnect).toHaveBeenCalledTimes(2);
  });

  it("retries immediately when requested manually", () => {
    const onReconnect = vi.fn();
    const { result } = renderHook(() =>
      useWorkspaceWsReconnect({
        enabled: true,
        onReconnect,
        closeCode: 1006,
      }),
    );

    act(() => {
      result.current.retryNow();
    });

    expect(onReconnect).toHaveBeenCalledTimes(1);
    expect(result.current.attemptCount).toBe(1);
  });

  it("keeps the reconnect banner state while a retry attempt is connecting", () => {
    const onReconnect = vi.fn();
    const { result, rerender } = renderHook(
      ({ enabled }) =>
        useWorkspaceWsReconnect({
          enabled,
          onReconnect,
          closeCode: 1006,
        }),
      { initialProps: { enabled: true } },
    );

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(result.current.isReconnecting).toBe(true);
    expect(result.current.attemptCount).toBe(1);

    rerender({ enabled: false });

    expect(result.current.isReconnecting).toBe(true);
    expect(result.current.attemptCount).toBe(1);
  });
});

function setDocumentHidden(value: boolean) {
  Object.defineProperty(document, "hidden", {
    configurable: true,
    value,
  });
}
