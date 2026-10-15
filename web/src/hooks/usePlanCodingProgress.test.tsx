import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getPlanCodingProgress } from "../api/client";
import type { PlanCodingProgressDto } from "../api/types";
import { usePlanCodingProgress } from "./usePlanCodingProgress";

vi.mock("../api/client", () => ({
  getPlanCodingProgress: vi.fn(),
}));

function progressDto(planId: string): PlanCodingProgressDto {
  return {
    plan_id: planId,
    bound_plan_revision_id: "plan_revision_0001",
    command_id: null,
    mode: null,
    phase: "running",
    primary_attempt_id: "coding_attempt_busi_0001",
    selected_unit_id: null,
    selected_attempt_id: null,
    target_attempts: [],
    units: [],
    plan_complete: false,
  };
}

// P1-b（REQ-MTG-12）：统一 `{data,loading,error,refetch}` 返回（无兼容别名），
// 支持可选 pollIntervalMs 轮询——总览可见时 5 秒见非当前仓更新。
describe("usePlanCodingProgress unified query shape (P1-b/REQ-MTG-12)", () => {
  beforeEach(() => {
    vi.mocked(getPlanCodingProgress).mockReset();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("exposes {data,loading,error,refetch} and resolves data once fetched", async () => {
    vi.mocked(getPlanCodingProgress).mockResolvedValue(
      progressDto("issue_plan_0001"),
    );

    const { result } = renderHook(() =>
      usePlanCodingProgress({
        projectId: "project_0001",
        issueId: "issue_0001",
        planId: "issue_plan_0001",
      }),
    );

    expect(result.current.data).toBeNull();
    expect(result.current.loading).toBe(true);
    expect(result.current.error).toBeNull();
    expect(typeof result.current.refetch).toBe("function");

    await act(async () => {
      await Promise.resolve();
    });

    expect(result.current.data?.plan_id).toBe("issue_plan_0001");
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it("surfaces GET failure as error with null data instead of throwing", async () => {
    vi.mocked(getPlanCodingProgress).mockRejectedValue(new Error("boom"));

    const { result } = renderHook(() =>
      usePlanCodingProgress({
        projectId: "project_0001",
        issueId: "issue_0001",
        planId: "issue_plan_0001",
      }),
    );

    await act(async () => {
      await Promise.resolve();
    });

    expect(result.current.data).toBeNull();
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBe("boom");
  });

  it("refetch re-issues the GET for the same address", async () => {
    vi.mocked(getPlanCodingProgress).mockResolvedValue(
      progressDto("issue_plan_0001"),
    );

    const { result } = renderHook(() =>
      usePlanCodingProgress({
        projectId: "project_0001",
        issueId: "issue_0001",
        planId: "issue_plan_0001",
      }),
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(getPlanCodingProgress).toHaveBeenCalledTimes(1);

    await act(async () => {
      result.current.refetch();
      await Promise.resolve();
    });

    expect(getPlanCodingProgress).toHaveBeenCalledTimes(2);
    expect(getPlanCodingProgress).toHaveBeenLastCalledWith(
      "project_0001",
      "issue_0001",
      "issue_plan_0001",
    );
    expect(result.current.data?.plan_id).toBe("issue_plan_0001");
  });

  it("polls every pollIntervalMs while eligible and stops after unmount", async () => {
    vi.useFakeTimers();
    vi.mocked(getPlanCodingProgress).mockResolvedValue(
      progressDto("issue_plan_0001"),
    );

    const { unmount } = renderHook(() =>
      usePlanCodingProgress({
        projectId: "project_0001",
        issueId: "issue_0001",
        planId: "issue_plan_0001",
        pollIntervalMs: 5_000,
      }),
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(getPlanCodingProgress).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(4_999);
    });
    expect(getPlanCodingProgress).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(getPlanCodingProgress).toHaveBeenCalledTimes(2);

    unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getPlanCodingProgress).toHaveBeenCalledTimes(2);
  });

  it("stays idle without requests when the address is missing or disabled", async () => {
    const { result } = renderHook(() =>
      usePlanCodingProgress({
        projectId: "project_0001",
        issueId: "issue_0001",
        planId: null,
      }),
    );

    expect(result.current.data).toBeNull();
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeNull();

    const disabled = renderHook(() =>
      usePlanCodingProgress({
        projectId: "project_0001",
        issueId: "issue_0001",
        planId: "issue_plan_0001",
        enabled: false,
      }),
    );
    expect(disabled.result.current.data).toBeNull();

    await act(async () => {
      await Promise.resolve();
    });
    expect(getPlanCodingProgress).not.toHaveBeenCalled();
  });
});
