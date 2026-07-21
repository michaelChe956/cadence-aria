import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { deleteCodingAttempt } from "../api/client";
import type { CodingAttemptAddress } from "../api/types";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  deferred,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
} from "./CodingWorkspacePage.test-utils";

vi.mock("../api/client", () => ({
  confirmWorkItemExecutionPlan: vi.fn(),
  deleteCodingAttempt: vi.fn(),
  getCodingAttemptDiff: vi.fn(),
  requestWorkItemExecutionPlanChange: vi.fn(),
}));

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

describe("CodingWorkspacePage delete request isolation", () => {
  installCodingWorkspacePageTestHooks();

  const SAME_ATTEMPT_OTHER_SCOPE = {
    projectId: "project_0002",
    issueId: "issue_0002",
    attemptId: CODING_ATTEMPT_ADDRESS.attemptId,
  } as const;

  function setReadyAddress(address: CodingAttemptAddress) {
    useCodingWorkspaceStore.setState({
      projectId: address.projectId,
      issueId: address.issueId,
      attemptId: address.attemptId,
      status: "running",
      stage: "coding",
    });
  }

  it.each(["resolve", "reject"] as const)(
    "ignores a stale coding workspace delete %s after switching the full address",
    async (outcome) => {
      mockCodingWs();
      const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
      const pending = deferred<void>();
      vi.mocked(deleteCodingAttempt).mockReturnValue(pending.promise);
      const onBackA = vi.fn();
      const onBackB = vi.fn();
      setReadyAddress(CODING_ATTEMPT_ADDRESS);

      try {
        const view = render(
          <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBackA} />,
        );
        await userEvent.click(
          screen.getByRole("button", { name: "删除 Coding Workspace" }),
        );
        setReadyAddress(SAME_ATTEMPT_OTHER_SCOPE);
        view.rerender(
          <CodingWorkspacePage address={SAME_ATTEMPT_OTHER_SCOPE} onBack={onBackB} />,
        );

        await act(async () => {
          if (outcome === "resolve") {
            pending.resolve(undefined);
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
      } finally {
        confirm.mockRestore();
      }
    },
  );

  it("keeps the current address delete busy when an older delete settles", async () => {
    mockCodingWs();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const pendingA = deferred<void>();
    const pendingB = deferred<void>();
    vi.mocked(deleteCodingAttempt)
      .mockReturnValueOnce(pendingA.promise)
      .mockReturnValueOnce(pendingB.promise);
    const onBackA = vi.fn();
    const onBackB = vi.fn();
    setReadyAddress(CODING_ATTEMPT_ADDRESS);

    try {
      const view = render(
        <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={onBackA} />,
      );
      await userEvent.click(
        screen.getByRole("button", { name: "删除 Coding Workspace" }),
      );
      setReadyAddress(SAME_ATTEMPT_OTHER_SCOPE);
      view.rerender(
        <CodingWorkspacePage address={SAME_ATTEMPT_OTHER_SCOPE} onBack={onBackB} />,
      );
      const deleteButton = screen.getByRole("button", {
        name: "删除 Coding Workspace",
      });
      await waitFor(() => expect(deleteButton).toBeEnabled());
      await userEvent.click(deleteButton);
      expect(deleteButton).toBeDisabled();
      expect(deleteCodingAttempt).toHaveBeenNthCalledWith(2, SAME_ATTEMPT_OTHER_SCOPE);

      await act(async () => {
        pendingA.resolve(undefined);
        await pendingA.promise;
      });
      expect(onBackA).not.toHaveBeenCalled();
      expect(onBackB).not.toHaveBeenCalled();
      expect(deleteButton).toBeDisabled();

      await act(async () => {
        pendingB.resolve(undefined);
        await pendingB.promise;
      });
      expect(onBackB).toHaveBeenCalledTimes(1);
    } finally {
      confirm.mockRestore();
    }
  });
});
