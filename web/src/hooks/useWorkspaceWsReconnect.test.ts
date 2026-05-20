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

  it("pauses while document is hidden", () => {
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
      vi.advanceTimersByTime(5000);
    });

    expect(onReconnect).not.toHaveBeenCalled();
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
});

function setDocumentHidden(value: boolean) {
  Object.defineProperty(document, "hidden", {
    configurable: true,
    value,
  });
}
