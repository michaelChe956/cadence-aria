import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  ApiRequestError,
  getLegacyCodingAttemptSnapshot,
} from "../api/client";
import type { CodingAttemptSnapshotResponse } from "../api/types";
import { LegacyCodingWorkspaceRedirect } from "./LegacyCodingWorkspaceRedirect";

vi.mock("../api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api/client")>();
  return {
    ...actual,
    getLegacyCodingAttemptSnapshot: vi.fn(),
  };
});

function snapshotResponse(
  attemptId = "coding_attempt_0001",
): CodingAttemptSnapshotResponse {
  return {
    attempt: {
      project_id: "project_0001",
      issue_id: "issue_0001",
      attempt_id: attemptId,
    },
  } as CodingAttemptSnapshotResponse;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("LegacyCodingWorkspaceRedirect", () => {
  beforeEach(() => {
    vi.mocked(getLegacyCodingAttemptSnapshot).mockReset();
  });

  it("resolves a unique legacy attempt to the scoped address", async () => {
    vi.mocked(getLegacyCodingAttemptSnapshot).mockResolvedValue(
      snapshotResponse(),
    );
    const onResolved = vi.fn();

    render(
      <LegacyCodingWorkspaceRedirect
        attemptId="coding_attempt_0001"
        onResolved={onResolved}
        onBack={vi.fn()}
      />,
    );

    await waitFor(() =>
      expect(onResolved).toHaveBeenCalledWith({
        projectId: "project_0001",
        issueId: "issue_0001",
        attemptId: "coding_attempt_0001",
      }),
    );
  });

  it("shows an actionable message for an ambiguous legacy attempt", async () => {
    vi.mocked(getLegacyCodingAttemptSnapshot).mockRejectedValue(
      new ApiRequestError({
        code: "coding_attempt_ambiguous",
        message: "coding attempt matches multiple issues",
        details: { attempt_id: "coding_attempt_0001" },
      }),
    );
    const onBack = vi.fn();

    render(
      <LegacyCodingWorkspaceRedirect
        attemptId="coding_attempt_0001"
        onResolved={vi.fn()}
        onBack={onBack}
      />,
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "该历史 Coding Attempt ID 对应多个 Issue",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "返回 Workbench" }),
    );
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it.each([
    [
      "not found",
      new ApiRequestError({
        code: "coding_attempt_not_found",
        message: "coding attempt not found",
        details: { attempt_id: "coding_attempt_0001" },
      }),
    ],
    ["other errors", new Error("network unavailable")],
  ])("shows the not-found guidance for %s", async (_name, reason) => {
    vi.mocked(getLegacyCodingAttemptSnapshot).mockRejectedValue(reason);

    render(
      <LegacyCodingWorkspaceRedirect
        attemptId="coding_attempt_0001"
        onResolved={vi.fn()}
        onBack={vi.fn()}
      />,
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Coding Attempt 不存在或已删除",
    );
    expect(
      screen.getByRole("button", { name: "返回 Workbench" }),
    ).toBeEnabled();
  });

  it("ignores a stale request after the attempt id changes", async () => {
    const firstRequest = deferred<CodingAttemptSnapshotResponse>();
    const secondRequest = deferred<CodingAttemptSnapshotResponse>();
    vi.mocked(getLegacyCodingAttemptSnapshot)
      .mockReturnValueOnce(firstRequest.promise)
      .mockReturnValueOnce(secondRequest.promise);
    const onResolved = vi.fn();
    const onBack = vi.fn();

    const { rerender } = render(
      <LegacyCodingWorkspaceRedirect
        attemptId="coding_attempt_0001"
        onResolved={onResolved}
        onBack={onBack}
      />,
    );
    rerender(
      <LegacyCodingWorkspaceRedirect
        attemptId="coding_attempt_0002"
        onResolved={onResolved}
        onBack={onBack}
      />,
    );

    await act(async () => {
      secondRequest.resolve(snapshotResponse("coding_attempt_0002"));
      await secondRequest.promise;
    });
    await waitFor(() =>
      expect(onResolved).toHaveBeenCalledWith({
        projectId: "project_0001",
        issueId: "issue_0001",
        attemptId: "coding_attempt_0002",
      }),
    );

    await act(async () => {
      firstRequest.resolve(snapshotResponse("coding_attempt_0001"));
      await firstRequest.promise;
    });
    expect(onResolved).toHaveBeenCalledTimes(1);
  });

  it("clears a previous error when the attempt id changes", async () => {
    const secondRequest = deferred<CodingAttemptSnapshotResponse>();
    vi.mocked(getLegacyCodingAttemptSnapshot)
      .mockRejectedValueOnce(
        new ApiRequestError({
          code: "coding_attempt_ambiguous",
          message: "coding attempt matches multiple issues",
          details: { attempt_id: "coding_attempt_0001" },
        }),
      )
      .mockReturnValueOnce(secondRequest.promise);
    const onResolved = vi.fn();
    const onBack = vi.fn();

    const { rerender } = render(
      <LegacyCodingWorkspaceRedirect
        attemptId="coding_attempt_0001"
        onResolved={onResolved}
        onBack={onBack}
      />,
    );
    expect(await screen.findByRole("alert")).toBeInTheDocument();

    rerender(
      <LegacyCodingWorkspaceRedirect
        attemptId="coding_attempt_0002"
        onResolved={onResolved}
        onBack={onBack}
      />,
    );

    expect(screen.getByRole("status")).toHaveTextContent(
      "正在定位 Coding Attempt",
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    await act(async () => {
      secondRequest.resolve(snapshotResponse("coding_attempt_0002"));
      await secondRequest.promise;
    });
  });

  it("does not resolve after unmount", async () => {
    const request = deferred<CodingAttemptSnapshotResponse>();
    vi.mocked(getLegacyCodingAttemptSnapshot).mockReturnValue(request.promise);
    const onResolved = vi.fn();

    const { unmount } = render(
      <LegacyCodingWorkspaceRedirect
        attemptId="coding_attempt_0001"
        onResolved={onResolved}
        onBack={vi.fn()}
      />,
    );
    unmount();

    await act(async () => {
      request.resolve(snapshotResponse());
      await request.promise;
    });
    expect(onResolved).not.toHaveBeenCalled();
  });
});
