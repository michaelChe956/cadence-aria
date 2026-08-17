import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { IssueDeliverySummaryDto } from "../../api/types";
import { DeliveryStatusPanel } from "./DeliveryStatusPanel";

const allPushedSummary: IssueDeliverySummaryDto = {
  project_id: "project_0001",
  issue_id: "issue_0001",
  overall: "all_pushed",
  entries: [
    {
      repository_name: "cadence-aria",
      work_item_id: "work_item_0001",
      attempt_status: "completed",
      branch_name: "feat/delivery",
      commit_sha: "abc123def456",
      push_status: "pushed",
      push_error: null,
    },
    {
      repository_name: "cadence-web",
      work_item_id: "work_item_0002",
      attempt_status: "completed",
      branch_name: "feat/delivery-web",
      commit_sha: "fedcba654321",
      push_status: "pushed",
      push_error: null,
    },
  ],
};

const partialSummary: IssueDeliverySummaryDto = {
  project_id: "project_0001",
  issue_id: "issue_0001",
  overall: "partial",
  entries: [
    {
      repository_name: "cadence-aria",
      work_item_id: "work_item_0001",
      attempt_status: "completed",
      branch_name: "feat/delivery",
      commit_sha: "abc123def456",
      push_status: "pushed",
      push_error: null,
    },
    {
      repository_name: "cadence-web",
      work_item_id: "work_item_0002",
      attempt_status: "completed",
      branch_name: "feat/delivery-web",
      commit_sha: "fedcba654321",
      push_status: "failed",
      push_error: "remote rejected: non-fast-forward",
    },
  ],
};

const notPushedSummary: IssueDeliverySummaryDto = {
  project_id: "project_0001",
  issue_id: "issue_0001",
  overall: "partial",
  entries: [
    {
      repository_name: "cadence-aria",
      work_item_id: "work_item_0003",
      attempt_status: "completed",
      branch_name: "feat/delivery-local",
      commit_sha: "abc123def456",
      push_status: "not_pushed",
      push_error: null,
    },
  ],
};

const noneSummary: IssueDeliverySummaryDto = {
  project_id: "project_0001",
  issue_id: "issue_0001",
  overall: "none",
  entries: [],
};

describe("DeliveryStatusPanel", () => {
  it("renders all_pushed with green badge", () => {
    render(<DeliveryStatusPanel summary={allPushedSummary} />);

    expect(screen.getByText("已全部交付")).toBeInTheDocument();
    expect(screen.getByTestId("delivery-status-badge")).toHaveAttribute(
      "data-status",
      "all_pushed",
    );
    expect(screen.getByText("cadence-aria")).toBeInTheDocument();
    expect(screen.getByText("feat/delivery")).toBeInTheDocument();
    expect(screen.getByText("cadence-web")).toBeInTheDocument();
    expect(screen.getByText("feat/delivery-web")).toBeInTheDocument();
    expect(screen.getAllByText("已推送")).toHaveLength(2);
  });

  it("renders partial with warning and failed row error text", () => {
    render(<DeliveryStatusPanel summary={partialSummary} />);

    expect(screen.getByText("部分交付")).toBeInTheDocument();
    expect(screen.getByTestId("delivery-status-badge")).toHaveAttribute(
      "data-status",
      "partial",
    );
    expect(
      screen.getByText("remote rejected: non-fast-forward"),
    ).toBeInTheDocument();

    const rows = screen.getAllByTestId("delivery-entry-row");
    expect(rows).toHaveLength(2);
    const failedRows = rows.filter(
      (row) => row.getAttribute("data-status") === "failed",
    );
    expect(failedRows).toHaveLength(1);
    expect(failedRows[0]).toHaveTextContent("cadence-web");
    expect(failedRows[0]).toHaveTextContent("remote rejected: non-fast-forward");
  });

  it("renders not_pushed as an explicit pending delivery branch", () => {
    render(<DeliveryStatusPanel summary={notPushedSummary} />);

    const row = screen.getByTestId("delivery-entry-row");
    expect(row).toHaveAttribute("data-status", "pending");
    expect(row).toHaveTextContent("cadence-aria");
    expect(row).toHaveTextContent("未推送");
    expect(row).not.toHaveTextContent("推送失败");
    expect(screen.queryByTestId("delivery-entry-error")).not.toBeInTheDocument();
  });

  it("renders none with unavailable hint", () => {
    render(<DeliveryStatusPanel summary={noneSummary} />);

    expect(screen.getByText("无 Work Item，不可交付")).toBeInTheDocument();
    expect(screen.getByTestId("delivery-status-badge")).toHaveAttribute(
      "data-status",
      "none",
    );
    expect(screen.queryAllByTestId("delivery-entry-row")).toHaveLength(0);
  });
});
