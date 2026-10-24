import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { deletePlanCodingWorkspaces } from "../api/client";
import type * as ApiClientModule from "../api/client";
import type {
  CodingAttemptAddress,
  PlanCodingWorkspaceDeletionResultDto,
} from "../api/types";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  deferred,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
} from "./CodingWorkspacePage.test-utils";

vi.mock("../api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof ApiClientModule>();
  return {
    ...actual,
    // 聚合缺席 → 进度条走 slim 分支（plan 级删除入口仍在）；真实 client 的
    // URL/ApiRequestError 语义保留，供 409 分类分支使用。
    getPlanCodingProgress: vi.fn(async () => {
      throw new Error("plan aggregate not configured for this test");
    }),
    deletePlanCodingWorkspaces: vi.fn(),
  };
});

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => <div>{value}</div>,
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({ modified }: { modified: string }) => <div>{modified}</div>,
}));

describe("CodingWorkspacePage group delete request isolation", () => {
  installCodingWorkspacePageTestHooks();

  const SAME_ATTEMPT_OTHER_SCOPE = {
    projectId: "project_0002",
    issueId: "issue_0002",
    attemptId: CODING_ATTEMPT_ADDRESS.attemptId,
  } as const;

  function resolveWith(): PlanCodingWorkspaceDeletionResultDto {
    // 结算值不被页面消费（await 结果忽略），仅借类型占位。
    return undefined as unknown as PlanCodingWorkspaceDeletionResultDto;
  }

  function setReadyGroupAddress(address: CodingAttemptAddress) {
    useCodingWorkspaceStore.setState({
      projectId: address.projectId,
      issueId: address.issueId,
      attemptId: address.attemptId,
      status: "running",
      stage: "coding",
      attemptScope: "work_item_group",
      workItemGroupId: "issue_plan_0001",
      units: [],
    });
  }

  it.each(["resolve", "reject"] as const)(
    "ignores a stale group delete %s after switching the full address",
    async (outcome) => {
      mockCodingWs();
      const pending = deferred<PlanCodingWorkspaceDeletionResultDto>();
      vi.mocked(deletePlanCodingWorkspaces).mockImplementation(() => pending.promise);
      const onBackA = vi.fn();
      const onBackB = vi.fn();
      setReadyGroupAddress(CODING_ATTEMPT_ADDRESS);

      const view = render(
        <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBackA} />,
      );
      await userEvent.click(
        await screen.findByTestId("plan-coding-delete-all"),
      );
      setReadyGroupAddress(SAME_ATTEMPT_OTHER_SCOPE);
      view.rerender(
        <CodingWorkspacePage address={SAME_ATTEMPT_OTHER_SCOPE} onBack={onBackB} />,
      );

      await act(async () => {
        if (outcome === "resolve") {
          pending.resolve(resolveWith());
        } else {
          pending.reject(new Error("stale A delete error"));
        }
        await pending.promise.catch(() => undefined);
      });

      expect(onBackA).not.toHaveBeenCalled();
      expect(onBackB).not.toHaveBeenCalled();
      expect(screen.queryByText("stale A delete error")).not.toBeInTheDocument();
      expect(useCodingWorkspaceStore.getState().projectId).toBe(
        SAME_ATTEMPT_OTHER_SCOPE.projectId,
      );
      expect(useCodingWorkspaceStore.getState().issueId).toBe(
        SAME_ATTEMPT_OTHER_SCOPE.issueId,
      );
    },
  );

  it("keeps the current address group delete busy when an older delete settles", async () => {
    mockCodingWs();
    const pendingA = deferred<PlanCodingWorkspaceDeletionResultDto>();
    const pendingB = deferred<PlanCodingWorkspaceDeletionResultDto>();
    vi.mocked(deletePlanCodingWorkspaces)
      .mockImplementationOnce(() => pendingA.promise)
      .mockImplementationOnce(() => pendingB.promise);
    const onBackA = vi.fn();
    const onBackB = vi.fn();
    setReadyGroupAddress(CODING_ATTEMPT_ADDRESS);

    const view = render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBackA} />,
    );
    await userEvent.click(await screen.findByTestId("plan-coding-delete-all"));
    setReadyGroupAddress(SAME_ATTEMPT_OTHER_SCOPE);
    view.rerender(
      <CodingWorkspacePage address={SAME_ATTEMPT_OTHER_SCOPE} onBack={onBackB} />,
    );
    const deleteButton = await screen.findByTestId("plan-coding-delete-all");
    // 地址切换 effect 复位 busy，新地址入口立即可用。
    await waitFor(() => expect(deleteButton).toBeEnabled());
    await userEvent.click(deleteButton);
    expect(deleteButton).toBeDisabled();
    expect(deletePlanCodingWorkspaces).toHaveBeenNthCalledWith(
      2,
      SAME_ATTEMPT_OTHER_SCOPE.projectId,
      SAME_ATTEMPT_OTHER_SCOPE.issueId,
      "issue_plan_0001",
      expect.objectContaining({ command_id: expect.any(String) }),
    );

    await act(async () => {
      pendingA.resolve(resolveWith());
      await pendingA.promise;
    });
    expect(onBackA).not.toHaveBeenCalled();
    expect(onBackB).not.toHaveBeenCalled();
    expect(deleteButton).toBeDisabled();

    await act(async () => {
      pendingB.resolve(resolveWith());
      await pendingB.promise;
    });
    expect(onBackB).toHaveBeenCalledTimes(1);
  });
});
