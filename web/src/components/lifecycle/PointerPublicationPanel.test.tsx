import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  PointerPublicationDto,
  PointerPublicationEntryDto,
} from "../../api/types";
import { PointerPublicationPanel } from "./PointerPublicationPanel";

function entry(
  memberRepoId: string,
  state: PointerPublicationEntryDto["state"],
  extra: Partial<PointerPublicationEntryDto> = {},
): PointerPublicationEntryDto {
  return {
    member_repo_id: memberRepoId,
    state,
    branch_name: null,
    commit_sha: null,
    push_error: null,
    conflict_detail: null,
    ...extra,
  };
}

function publication(
  status: PointerPublicationDto["status"],
  entries: PointerPublicationEntryDto[],
): PointerPublicationDto {
  return {
    id: "pub-0001",
    project_id: "project_0001",
    logical_codebase_id: "logical-0001",
    batch_kind: "full",
    entries,
    status,
    created_at: "2026-08-14T00:00:00Z",
    updated_at: "2026-08-14T00:00:01Z",
  };
}

function renderPanel(
  overrides: Partial<{
    publication: PointerPublicationDto | null;
    onPublishFull: () => void;
    onPublishIncremental: () => void;
    onRetryRepo: (memberRepoId: string) => void;
    onRevoke: () => void;
  }> = {},
) {
  const props = {
    publication: null as PointerPublicationDto | null,
    onPublishFull: () => {},
    onPublishIncremental: () => {},
    onRetryRepo: (_memberRepoId: string) => {},
    onRevoke: () => {},
    ...overrides,
  };
  return render(<PointerPublicationPanel {...props} />);
}

describe("PointerPublicationPanel", () => {
  it("renders completed_all with all-green entry list", () => {
    renderPanel({
      publication: publication("completed_all", [
        entry("repo-a", "pushed", {
          branch_name: "feat/pointer",
          commit_sha: "abc123def456",
        }),
        entry("repo-b", "review_created"),
      ]),
    });

    expect(screen.getByText("全部发布完成")).toBeInTheDocument();
    expect(screen.getByTestId("pointer-publication-badge")).toHaveAttribute(
      "data-status",
      "completed_all",
    );

    const rows = screen.getAllByTestId("pointer-publication-entry-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveAttribute("data-state", "pushed");
    expect(rows[1]).toHaveAttribute("data-state", "review_created");
    expect(screen.getByText("repo-a")).toBeInTheDocument();
    expect(screen.getByText("feat/pointer")).toBeInTheDocument();
  });

  it("renders completed_partial with warning hint", () => {
    renderPanel({
      publication: publication("completed_partial", [
        entry("repo-a", "pushed"),
        entry("repo-b", "failed", { push_error: "remote rejected" }),
      ]),
    });

    expect(screen.getByText("部分发布完成")).toBeInTheDocument();
    expect(screen.getByTestId("pointer-publication-badge")).toHaveAttribute(
      "data-status",
      "completed_partial",
    );
    expect(
      screen.getByText("部分成员发布失败或冲突，需人工处理后重试。"),
    ).toBeInTheDocument();
  });

  it("renders in_progress state with disabled publish button", () => {
    renderPanel({
      publication: publication("in_progress", [
        entry("repo-a", "pushed"),
        entry("repo-b", "pending"),
      ]),
    });

    expect(screen.getByText("发布中")).toBeInTheDocument();
    expect(screen.getByTestId("pointer-publication-badge")).toHaveAttribute(
      "data-status",
      "in_progress",
    );
    expect(screen.getByRole("button", { name: "全量发布" })).toBeDisabled();
  });

  it("conflict entry shows conflict point and retry button triggers retry", () => {
    const onRetryRepo = vi.fn();
    renderPanel({
      publication: publication("completed_partial", [
        entry("repo-c", "conflict", {
          conflict_detail: "指针块不一致：期望 canonical-123，实际 legacy-456",
        }),
      ]),
      onRetryRepo,
    });

    expect(
      screen.getByText("指针块不一致：期望 canonical-123，实际 legacy-456"),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "人工已解决，重试" }),
    );
    expect(onRetryRepo).toHaveBeenCalledTimes(1);
    expect(onRetryRepo).toHaveBeenCalledWith("repo-c");
  });

  it("failed entry shows push_error and retry button triggers retry", () => {
    const onRetryRepo = vi.fn();
    renderPanel({
      publication: publication("completed_partial", [
        entry("repo-d", "failed", {
          push_error: "remote rejected: non-fast-forward",
        }),
      ]),
      onRetryRepo,
    });

    expect(
      screen.getByText("remote rejected: non-fast-forward"),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    expect(onRetryRepo).toHaveBeenCalledTimes(1);
    expect(onRetryRepo).toHaveBeenCalledWith("repo-d");
  });

  it("revoke requires confirmation before calling revoke", () => {
    const onRevoke = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderPanel({
      publication: publication("completed_all", [entry("repo-a", "pushed")]),
      onRevoke,
    });

    fireEvent.click(screen.getByRole("button", { name: "撤回" }));
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(onRevoke).not.toHaveBeenCalled();

    confirmSpy.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "撤回" }));
    expect(onRevoke).toHaveBeenCalledTimes(1);

    confirmSpy.mockRestore();
  });

  it("renders revoked status and revoked entries", () => {
    renderPanel({
      publication: publication("revoked", [entry("repo-a", "revoked")]),
    });

    expect(screen.getAllByText("已撤回").length).toBeGreaterThan(0);
    expect(screen.getByTestId("pointer-publication-badge")).toHaveAttribute(
      "data-status",
      "revoked",
    );
    expect(screen.getByTestId("pointer-publication-entry-row")).toHaveAttribute(
      "data-state",
      "revoked",
    );
    expect(
      screen.queryByRole("button", { name: "撤回" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "全量发布" }),
    ).not.toBeInTheDocument();
  });

  it("renders empty state with publish full button", () => {
    const onPublishFull = vi.fn();
    renderPanel({ publication: null, onPublishFull });

    expect(screen.getByText(/尚无指针发布记录/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "全量发布" }));
    expect(onPublishFull).toHaveBeenCalledTimes(1);
  });

  it("triggers publish full when publish button clicked", () => {
    const onPublishFull = vi.fn();
    renderPanel({
      publication: publication("completed_all", []),
      onPublishFull,
    });

    fireEvent.click(screen.getByRole("button", { name: "全量发布" }));
    expect(onPublishFull).toHaveBeenCalledTimes(1);
  });

  it("triggers incremental publish when incremental button clicked", () => {
    const onPublishIncremental = vi.fn();
    renderPanel({
      publication: publication("completed_all", []),
      onPublishIncremental,
    });

    fireEvent.click(screen.getByRole("button", { name: "增量发布" }));
    expect(onPublishIncremental).toHaveBeenCalledTimes(1);
  });
});
