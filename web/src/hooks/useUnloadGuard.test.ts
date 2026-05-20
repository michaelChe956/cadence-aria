import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useBlocker } from "@tanstack/react-router";
import { useUnloadGuard } from "./useUnloadGuard";

vi.mock("@tanstack/react-router", () => ({
  useBlocker: vi.fn(),
}));

describe("useUnloadGuard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("registers beforeunload and a navigation blocker when enabled", () => {
    const addEventListener = vi.spyOn(window, "addEventListener");

    renderHook(() =>
      useUnloadGuard({
        enabled: true,
        message: "运行中，离开将中止",
      }),
    );

    expect(addEventListener).toHaveBeenCalledWith("beforeunload", expect.any(Function));
    expect(useBlocker).toHaveBeenCalledWith({
      condition: true,
      blockerFn: expect.any(Function),
    });

    addEventListener.mockRestore();
  });

  it("does not register beforeunload when disabled", () => {
    const addEventListener = vi.spyOn(window, "addEventListener");

    renderHook(() =>
      useUnloadGuard({
        enabled: false,
        message: "运行中，离开将中止",
      }),
    );

    expect(addEventListener).not.toHaveBeenCalled();
    expect(useBlocker).toHaveBeenCalledWith({
      condition: false,
      blockerFn: expect.any(Function),
    });

    addEventListener.mockRestore();
  });

  it("writes returnValue for native beforeunload prompts", () => {
    const addEventListener = vi.spyOn(window, "addEventListener");

    renderHook(() =>
      useUnloadGuard({
        enabled: true,
        message: "运行中，离开将中止",
      }),
    );

    const handler = addEventListener.mock.calls.find(([type]) => type === "beforeunload")?.[1] as
      | ((event: BeforeUnloadEvent) => string)
      | undefined;
    const event = {
      preventDefault: vi.fn(),
      returnValue: "",
    } as unknown as BeforeUnloadEvent;

    const result = handler?.(event);

    expect(event.preventDefault).toHaveBeenCalled();
    expect(event.returnValue).toBe("运行中，离开将中止");
    expect(result).toBe("运行中，离开将中止");

    addEventListener.mockRestore();
  });
});
