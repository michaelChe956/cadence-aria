import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { confirmPlanCoding } from "../api/client";
import type { PlanCodingFinalConfirmResultDto } from "../api/types";
import { PlanCodingFinalConfirm } from "./PlanCodingFinalConfirm";
import type { PlanCodingFinalConfirmView } from "./plan-coding-progress";

vi.mock("../api/client", () => ({
  confirmPlanCoding: vi.fn(),
}));

function finalConfirmView(
  state: PlanCodingFinalConfirmView["state"],
  extra: Partial<PlanCodingFinalConfirmView> = {},
): PlanCodingFinalConfirmView {
  return {
    state,
    readyAttemptIds:
      state === "ready"
        ? [
            "coding_attempt_busi_0001",
            "coding_attempt_web_0001",
            "coding_attempt_gateway_0001",
            "coding_attempt_frontend_0001",
          ]
        : ["coding_attempt_web_0001", "coding_attempt_gateway_0001"],
    confirmedAttemptIds:
      state === "partial" ? ["coding_attempt_busi_0001"] : [],
    blocking: [],
    ...extra,
  };
}

function resultDto(
  overrides: Partial<PlanCodingFinalConfirmResultDto> = {},
): PlanCodingFinalConfirmResultDto {
  return {
    plan_id: "issue_plan_0001",
    outcome: "all_confirmed",
    results: [
      {
        attempt_id: "coding_attempt_busi_0001",
        target_repository_id: "repo_busi",
        status: "confirmed",
      },
      {
        attempt_id: "coding_attempt_web_0001",
        target_repository_id: "repo_web",
        status: "confirmed",
      },
    ],
    ...overrides,
  };
}

// P1-b（REQ-MTG-11）：plan 级聚合确认按钮——ready/partial 出现、一次点击仅一个
// POST、失败仓逐仓可见、done 只读成功、≥2 targets 才呈现。
describe("PlanCodingFinalConfirm aggregate confirm button (P1-b/REQ-MTG-11)", () => {
  const address = {
    projectId: "project_0001",
    issueId: "issue_0001",
    planId: "issue_plan_0001",
  } as const;

  beforeEach(() => {
    vi.mocked(confirmPlanCoding).mockReset();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the ready button with repo/unit counts and posts exactly once per click", async () => {
    vi.mocked(confirmPlanCoding).mockResolvedValue(resultDto());
    const onConfirmed = vi.fn();

    render(
      <PlanCodingFinalConfirm
        {...address}
        finalConfirm={finalConfirmView("ready")}
        targetCount={4}
        unitCount={5}
        onConfirmed={onConfirmed}
      />,
    );

    const button = screen.getByTestId("plan-final-confirm-button");
    expect(button).toHaveTextContent("确认本计划全部完成（4 仓/5 项）");

    fireEvent.click(button);
    await waitFor(() =>
      expect(screen.getByTestId("plan-final-confirm-result")).toBeInTheDocument(),
    );

    expect(confirmPlanCoding).toHaveBeenCalledTimes(1);
    expect(confirmPlanCoding).toHaveBeenCalledWith(
      "project_0001",
      "issue_0001",
      "issue_plan_0001",
    );
    expect(onConfirmed).toHaveBeenCalledTimes(1);

    const result = screen.getByTestId("plan-final-confirm-result");
    expect(result).toHaveTextContent("repo_busi");
    expect(result).toHaveTextContent("已确认");
    expect(result).toHaveTextContent("repo_web");
  });

  it("guards against duplicate POSTs while a request is in flight", async () => {
    let resolvePost: (value: PlanCodingFinalConfirmResultDto) => void = () => {};
    vi.mocked(confirmPlanCoding).mockReturnValue(
      new Promise<PlanCodingFinalConfirmResultDto>((resolve) => {
        resolvePost = resolve;
      }),
    );

    render(
      <PlanCodingFinalConfirm
        {...address}
        finalConfirm={finalConfirmView("ready")}
        targetCount={4}
        unitCount={5}
      />,
    );
    const button = screen.getByTestId("plan-final-confirm-button");
    fireEvent.click(button);
    fireEvent.click(button);
    fireEvent.click(button);

    expect(confirmPlanCoding).toHaveBeenCalledTimes(1);
    expect(button).toBeDisabled();
    expect(button).toHaveTextContent("确认中…");

    resolvePost(resultDto());
    await waitFor(() =>
      expect(screen.getByTestId("plan-final-confirm-result")).toBeInTheDocument(),
    );
  });

  it("shows per-repo results with the failed repo reason on partial failure", async () => {
    vi.mocked(confirmPlanCoding).mockResolvedValue(
      resultDto({
        outcome: "partially_confirmed",
        results: [
          {
            attempt_id: "coding_attempt_busi_0001",
            target_repository_id: "repo_busi",
            status: "confirmed",
          },
          {
            attempt_id: "coding_attempt_web_0001",
            target_repository_id: "repo_web",
            status: "failed",
            error_code: "worktree_dirty",
            message: "worktree has uncommitted changes",
          },
          {
            attempt_id: "coding_attempt_gateway_0001",
            target_repository_id: "repo_gateway",
            status: "not_attempted",
          },
        ],
      }),
    );

    render(
      <PlanCodingFinalConfirm
        {...address}
        finalConfirm={finalConfirmView("ready")}
        targetCount={4}
        unitCount={5}
      />,
    );

    fireEvent.click(screen.getByTestId("plan-final-confirm-button"));
    await waitFor(() =>
      expect(screen.getByTestId("plan-final-confirm-result")).toBeInTheDocument(),
    );

    const failed = screen.getByTestId("plan-final-confirm-result-repo_web");
    expect(failed).toHaveTextContent("repo_web");
    expect(failed).toHaveTextContent("失败");
    expect(failed).toHaveTextContent("worktree_dirty");
    expect(failed).toHaveTextContent("worktree has uncommitted changes");
    expect(screen.getByTestId("plan-final-confirm-result-repo_gateway")).toHaveTextContent(
      "未执行",
    );
  });

  it("renders the partial-state button scoped to the remaining ready repos", () => {
    render(
      <PlanCodingFinalConfirm
        {...address}
        finalConfirm={finalConfirmView("partial")}
        targetCount={4}
        unitCount={5}
      />,
    );

    expect(screen.getByTestId("plan-final-confirm-button")).toHaveTextContent(
      "确认本计划全部完成（2 仓/5 项）",
    );
  });

  it("renders read-only success for done and never posts", () => {
    render(
      <PlanCodingFinalConfirm
        {...address}
        finalConfirm={finalConfirmView("done", {
          readyAttemptIds: [],
          confirmedAttemptIds: [
            "coding_attempt_busi_0001",
            "coding_attempt_web_0001",
            "coding_attempt_gateway_0001",
            "coding_attempt_frontend_0001",
          ],
        })}
        targetCount={4}
        unitCount={5}
      />,
    );

    expect(screen.getByTestId("plan-final-confirm-done")).toHaveTextContent(
      "全部人工确认",
    );
    expect(screen.queryByTestId("plan-final-confirm-button")).toBeNull();
    expect(confirmPlanCoding).not.toHaveBeenCalled();
  });

  it("hides the aggregate button when not ready or below two targets", () => {
    const { rerender } = render(
      <PlanCodingFinalConfirm
        {...address}
        finalConfirm={finalConfirmView("not_ready", {
          readyAttemptIds: [],
          blocking: [
            {
              attemptId: "coding_attempt_web_0001",
              reasonCode: "attempt_not_in_final_confirm",
              message: "stage is not final_confirm",
            },
          ],
        })}
        targetCount={4}
        unitCount={5}
      />,
    );
    expect(screen.queryByTestId("plan-final-confirm-button")).toBeNull();

    rerender(
      <PlanCodingFinalConfirm
        {...address}
        finalConfirm={finalConfirmView("ready")}
        targetCount={1}
        unitCount={1}
      />,
    );
    expect(screen.queryByTestId("plan-final-confirm-button")).toBeNull();
    expect(confirmPlanCoding).not.toHaveBeenCalled();
  });

  it("surfaces a POST transport failure without losing the retry button", async () => {
    vi.mocked(confirmPlanCoding).mockRejectedValueOnce(
      new Error("network down"),
    );

    render(
      <PlanCodingFinalConfirm
        {...address}
        finalConfirm={finalConfirmView("ready")}
        targetCount={4}
        unitCount={5}
      />,
    );

    const button = screen.getByTestId("plan-final-confirm-button");
    fireEvent.click(button);
    await waitFor(() =>
      expect(screen.getByTestId("plan-final-confirm-error")).toHaveTextContent(
        "network down",
      ),
    );
    expect(button).toBeEnabled();
  });
});
