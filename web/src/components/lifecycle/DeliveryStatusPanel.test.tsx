import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { DeliveryEntryDto, IssueDeliverySummaryDto } from "../../api/types";
import { DeliveryStatusPanel } from "./DeliveryStatusPanel";

// owner 直批（2026-10-14）：5 个 Work Item 按 target 仓展开导致 frontend 重复两次；
// 期望去重为 4 个唯一仓（busi/api/gateway/frontend），per-WI 状态下沉为副行。
function deliveryEntry(
  repositoryName: string,
  workItemId: string,
  pushStatus: DeliveryEntryDto["push_status"],
): DeliveryEntryDto {
  return {
    repository_name: repositoryName,
    work_item_id: workItemId,
    attempt_status: "completed",
    branch_name: `feat/${workItemId}`,
    commit_sha: `${workItemId}sha`,
    push_status: pushStatus,
    push_error: pushStatus === "failed" ? "remote rejected" : null,
  };
}

const dedupedSummary: IssueDeliverySummaryDto = {
  project_id: "project_0001",
  issue_id: "issue_0001",
  overall: "partial",
  entries: [
    deliveryEntry("busi", "work_item_busi", "pushed"),
    deliveryEntry("api", "work_item_api", "pushed"),
    deliveryEntry("gateway", "work_item_gateway", "pushed"),
    deliveryEntry("frontend", "work_item_frontend_a", "pushed"),
    deliveryEntry("frontend", "work_item_frontend_b", "not_pushed"),
  ],
};

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

// FE-1：未配置远端的本地交付（skipped_not_configured）是完成态而非失败态，
// 但绝不能显示成「已推送」。
const localDeliverySummary: IssueDeliverySummaryDto = {
  project_id: "project_0001",
  issue_id: "issue_0001",
  overall: "partial",
  entries: [
    deliveryEntry("cadence-aria", "work_item_0001", "pushed"),
    deliveryEntry("cadence-web", "work_item_0002", "skipped_not_configured"),
  ],
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

  it("skipped_not_configured 渲染为本地交付，不冒充已推送也不计失败", () => {
    render(<DeliveryStatusPanel summary={localDeliverySummary} />);

    const rows = screen.getAllByTestId("delivery-entry-row");
    expect(rows).toHaveLength(2);
    const localRow = rows.find(
      (row) => row.getAttribute("data-status") === "skipped",
    );
    expect(localRow).toBeDefined();
    expect(localRow).toHaveTextContent("cadence-web");
    expect(localRow).toHaveTextContent("本地交付(无远端)");
    // 「已推送」只属于真正推送成功的仓卡。
    expect(screen.getAllByText("已推送")).toHaveLength(1);
    expect(screen.queryByTestId("delivery-entry-error")).not.toBeInTheDocument();
    expect(screen.queryByText(/推送失败/u)).not.toBeInTheDocument();
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

  it("按仓库去重：同一仓库的多个 Work Item 合并为一张仓卡并标注 WI 数", () => {
    render(<DeliveryStatusPanel summary={dedupedSummary} />);

    const rows = screen.getAllByTestId("delivery-entry-row");
    expect(rows).toHaveLength(4);
    // 仓名唯一：frontend 只出现一次。
    expect(screen.getAllByText("frontend")).toHaveLength(1);
    const frontend = rows.find(
      (row) => row.getAttribute("data-work-item-count") === "2",
    );
    expect(frontend).toBeDefined();
    expect(frontend).toHaveTextContent("2 个 Work Item");
    // 仓内出现未推送 -> 聚合状态取未推送，不误报已推送。
    expect(frontend).toHaveAttribute("data-status", "pending");
    expect(frontend).toHaveTextContent("已推送 1 · 未推送 1");
    // 单 WI 仓卡不渲染 WI 数副行。
    expect(rows.filter((row) => row.getAttribute("data-work-item-count") === "1"))
      .toHaveLength(3);
    expect(screen.queryAllByTestId("delivery-entry-work-item-count")).toHaveLength(1);
  });

  it("未传折叠控件时不渲染切换按钮（显式详情面常驻展开）", () => {
    render(<DeliveryStatusPanel summary={dedupedSummary} />);

    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getAllByTestId("delivery-entry-row")).toHaveLength(4);
  });

  it("折叠受控：collapsed 时只保留徽标与收起摘要，仓卡不渲染", () => {
    render(
      <DeliveryStatusPanel
        summary={dedupedSummary}
        collapsed
        onToggleCollapsed={() => {}}
      />,
    );

    expect(screen.getByTestId("delivery-status-badge")).toHaveTextContent(
      "部分交付",
    );
    expect(screen.queryAllByTestId("delivery-entry-row")).toHaveLength(0);
    expect(screen.getByTestId("delivery-status-collapsed-summary")).toHaveTextContent(
      "4 个仓库 · 5 个 Work Item",
    );
    expect(
      screen.getByRole("button", { name: /^展开交付明细/u }),
    ).toHaveAttribute("aria-expanded", "false");
  });

  it("展开态渲染仓卡明细，切换按钮可回调收起", async () => {
    const onToggleCollapsed = vi.fn();
    const user = userEvent.setup();
    render(
      <DeliveryStatusPanel
        summary={dedupedSummary}
        collapsed={false}
        onToggleCollapsed={onToggleCollapsed}
      />,
    );

    expect(screen.getAllByTestId("delivery-entry-row")).toHaveLength(4);
    const toggle = screen.getByRole("button", { name: "收起交付明细" });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    await user.click(toggle);
    expect(onToggleCollapsed).toHaveBeenCalledTimes(1);
  });
});
