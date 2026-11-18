import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AggregateIndexActiveResponse } from "../../api/types";
import { AggregateIndexCard } from "./AggregateIndexCard";

function renderCard(
  index: AggregateIndexActiveResponse,
  rebuilding = false,
  onRebuild = vi.fn(),
) {
  render(
    <AggregateIndexCard
      index={index}
      rebuilding={rebuilding}
      onRebuild={onRebuild}
    />,
  );
  return onRebuild;
}

describe("AggregateIndexCard", () => {
  it.each([
    ["active", "索引可用"],
    ["stale", "索引已过期"],
    ["degraded", "索引降级"],
    ["rebuilding", "正在重建索引"],
    ["missing", "尚未建立索引"],
  ] as const)("renders %s state with its fixed label", (state, label) => {
    renderCard({ state, revision: 7, indexed_at: "2026-08-18T00:00:00Z" });

    expect(screen.getByTestId("aggregate-index-status")).toHaveTextContent(
      state,
    );
    expect(screen.getByText(label)).toBeInTheDocument();
  });

  it("renders degraded warning and disables rebuild while synchronous rebuild is pending", async () => {
    const onRebuild = vi.fn();
    render(<AggregateIndexCard index={{ state: "degraded", revision: 7, indexed_at: "2026-08-18T00:00:00Z", warning: "sync failed" }} rebuilding onRebuild={onRebuild} />);
    expect(screen.getByTestId("aggregate-index-status")).toHaveTextContent("degraded");
    expect(screen.getByText("sync failed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重建索引" })).toBeDisabled();
    expect(screen.getByTestId("aggregate-index-spinner")).toBeInTheDocument();
  });

  it("keeps rebuild available when the aggregate index is missing", () => {
    const onRebuild = renderCard({ state: "missing" });

    fireEvent.click(screen.getByRole("button", { name: "重建索引" }));

    expect(onRebuild).toHaveBeenCalledOnce();
  });

  it("renders the index-unavailable failure face with its own rebuild action", () => {
    const onRebuild = vi.fn();
    render(
      <AggregateIndexCard
        index={{
          state: "active",
          revision: 7,
          indexed_at: "2026-08-18T00:00:00Z",
          evidence_copy: {
            title_zh: "聚合索引不可用，跨仓证据检索无法执行",
            reason_zh: "这不是「没有找到证据」：跨仓证据检索要读聚合索引。",
            action_zh: "在逻辑代码库管理面板的「聚合索引」处点「重建索引」重建聚合索引。",
            triage: "environment",
            machine_line: "evidence_index_unavailable",
          },
        }}
        rebuilding={false}
        onRebuild={onRebuild}
      />,
    );

    expect(
      screen.getByTestId("aggregate-index-evidence-unavailable"),
    ).toHaveTextContent("聚合索引不可用，跨仓证据检索无法执行");
    expect(
      screen.getByTestId("aggregate-index-evidence-title"),
    ).toHaveTextContent("聚合索引不可用");
    expect(
      screen.getByTestId("aggregate-index-evidence-unavailable"),
    ).toHaveTextContent("evidence_index_unavailable");

    const rebuild = screen.getByRole("button", { name: "重建索引" });
    fireEvent.click(rebuild);

    expect(onRebuild).toHaveBeenCalledOnce();
  });

  it("renders the aggregate-index failure face with its own rebuild action", () => {
    const onRebuild = vi.fn();
    render(
      <AggregateIndexCard
        index={{
          state: "active",
          revision: 7,
          indexed_at: "2026-08-18T00:00:00Z",
          warning:
            "aggregate_index_failed:aggregate_index_member_coverage_failed: no indexable code file under included members [gray-config, msha-sdk]",
          failure_copy: {
            title_zh: "成员仓里没有可索引的代码文件",
            reason_zh:
              "被纳入的成员 gray-config、msha-sdk 下找不到可索引的源码文件。",
            action_zh:
              "把真实源码放进成员仓（.js/.ts/.py/.go/.rs 等）；.html/.json/.md 不算源码。放好后点「重试」。",
            triage: "environment",
            machine_line: "aggregate_index_member_coverage_failed",
          },
        }}
        rebuilding={false}
        onRebuild={onRebuild}
      />,
    );

    expect(screen.getByTestId("aggregate-index-failure")).toHaveTextContent(
      "成员仓里没有可索引的代码文件",
    );
    expect(screen.getByTestId("aggregate-index-failure-title")).toHaveTextContent(
      "成员仓里没有可索引的代码文件",
    );
    expect(
      screen.getByText(
        "原因：被纳入的成员 gray-config、msha-sdk 下找不到可索引的源码文件。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "怎么做：把真实源码放进成员仓（.js/.ts/.py/.go/.rs 等）；.html/.json/.md 不算源码。放好后点「重试」。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("机器码：aggregate_index_member_coverage_failed"),
    ).toBeInTheDocument();

    // 原始英文串降级为失败面内的「诊断」次要行，只出现一次。
    expect(screen.getAllByTestId("aggregate-index-warning")).toHaveLength(1);
    expect(screen.getByTestId("aggregate-index-warning")).toHaveTextContent(
      "gray-config",
    );

    // 失败面自带上动作，标题行不再重复给出「重建索引」。
    const rebuild = screen.getAllByRole("button", { name: "重建索引" });
    expect(rebuild).toHaveLength(1);
    fireEvent.click(rebuild[0]);
    expect(onRebuild).toHaveBeenCalledOnce();
  });

  it("prefers the aggregate-index failure face over the index-unavailable face", () => {
    render(
      <AggregateIndexCard
        index={{
          state: "active",
          revision: 7,
          indexed_at: "2026-08-18T00:00:00Z",
          failure_copy: {
            title_zh: "成员仓里没有可索引的代码文件",
            reason_zh: "被纳入的成员 gray-config 下找不到可索引的源码文件。",
            action_zh: "把真实源码放进成员仓。",
            triage: "environment",
            machine_line: "aggregate_index_member_coverage_failed",
          },
          evidence_copy: {
            title_zh: "聚合索引不可用，跨仓证据检索无法执行",
            reason_zh: "这不是「没有找到证据」：跨仓证据检索要读聚合索引。",
            action_zh: "在逻辑代码库管理面板的「聚合索引」处点「重建索引」重建聚合索引。",
            triage: "environment",
            machine_line: "evidence_index_unavailable",
          },
        }}
        rebuilding={false}
        onRebuild={vi.fn()}
      />,
    );

    expect(screen.getByTestId("aggregate-index-failure")).toBeInTheDocument();
    expect(
      screen.queryByTestId("aggregate-index-evidence-unavailable"),
    ).toBeNull();
    expect(screen.getAllByRole("button", { name: "重建索引" })).toHaveLength(1);
  });

  it("keeps the index-unavailable face when there is no aggregate-index failure copy", () => {
    render(
      <AggregateIndexCard
        index={{
          state: "active",
          revision: 7,
          indexed_at: "2026-08-18T00:00:00Z",
          evidence_copy: {
            title_zh: "聚合索引不可用，跨仓证据检索无法执行",
            reason_zh: "这不是「没有找到证据」：跨仓证据检索要读聚合索引。",
            action_zh: "在逻辑代码库管理面板的「聚合索引」处点「重建索引」重建聚合索引。",
            triage: "environment",
            machine_line: "evidence_index_unavailable",
          },
        }}
        rebuilding={false}
        onRebuild={vi.fn()}
      />,
    );

    expect(
      screen.getByTestId("aggregate-index-evidence-unavailable"),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("aggregate-index-failure")).toBeNull();
    expect(screen.getAllByRole("button", { name: "重建索引" })).toHaveLength(1);
  });

  it("shows an actionable stale notice", () => {
    renderCard({
      state: "stale",
      revision: 6,
      indexed_at: "2026-08-18T00:00:00Z",
    });

    expect(screen.getByTestId("aggregate-index-stale-notice")).toHaveTextContent(
      "索引已过期，请重建后再使用聚合上下文。",
    );
  });
});
