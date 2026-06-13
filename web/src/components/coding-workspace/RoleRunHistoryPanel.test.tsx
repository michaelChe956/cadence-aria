import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { CodingRoleRun, CodingTimelineNode } from "../../api/types";
import { RoleRunHistoryPanel } from "./RoleRunHistoryPanel";

describe("RoleRunHistoryPanel", () => {
  it("renders run status, trigger, refs and node title", () => {
    render(
      <RoleRunHistoryPanel
        roleRuns={[
          roleRun({
            id: "coding_role_run_0001",
            role: "tester",
            stage: "testing",
            run_no: 1,
            status: "superseded",
            trigger: "initial",
            node_id: "coding_node_0003",
            superseded_by_run_id: "coding_role_run_0002",
            reason_code: "test_plan_missing_json",
            raw_provider_output_refs: ["provider-raw/testing/plan_tests_0001.txt"],
          }),
          roleRun({
            id: "coding_role_run_0002",
            role: "tester",
            stage: "testing",
            run_no: 2,
            status: "completed",
            trigger: "retry_test_plan",
            node_id: "coding_node_0004",
            artifact_refs: ["provider-raw/testing/testing_report_0002.json"],
          }),
        ]}
        timelineNodes={[
          node("coding_node_0003", "执行测试"),
          node("coding_node_0004", "执行测试重跑"),
        ]}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveTextContent("角色运行历史");
    expect(panel).toHaveTextContent("Tester #1");
    expect(panel).toHaveTextContent("已被替代");
    expect(panel).toHaveTextContent("initial");
    expect(panel).toHaveTextContent("test_plan_missing_json");
    expect(panel).toHaveTextContent("provider-raw/testing/plan_tests_0001.txt");
    expect(panel).toHaveTextContent("Tester #2");
    expect(panel).toHaveTextContent("已完成");
    expect(panel).toHaveTextContent("retry_test_plan");
    expect(panel).toHaveTextContent("执行测试重跑");
  });

  it("selects the linked timeline node", () => {
    const onSelectNode = vi.fn();
    render(
      <RoleRunHistoryPanel
        roleRuns={[roleRun({ node_id: "coding_node_0005" })]}
        timelineNodes={[node("coding_node_0005", "Analyst 路由决策")]}
        selectedNodeId={null}
        onSelectNode={onSelectNode}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Analyst #1/ }));

    expect(onSelectNode).toHaveBeenCalledWith("coding_node_0005");
  });
});

function roleRun(overrides: Partial<CodingRoleRun> = {}): CodingRoleRun {
  return {
    id: "coding_role_run_0001",
    attempt_id: "coding_attempt_0001",
    stage: "rework",
    role: "analyst",
    run_no: 1,
    status: "blocked",
    trigger: "retry_analyst",
    node_id: "coding_node_0005",
    started_at: "2026-06-13T00:00:00Z",
    completed_at: null,
    supersedes_run_id: null,
    superseded_by_run_id: null,
    reason_code: "analyst_human_gate",
    raw_provider_output_refs: [],
    artifact_refs: [],
    ...overrides,
  };
}

function node(id: string, title: string): CodingTimelineNode {
  return {
    id,
    attempt_id: "coding_attempt_0001",
    stage: "rework",
    title,
    status: "blocked",
    agent_role: "system",
    summary: null,
    started_at: "2026-06-13T00:00:00Z",
    completed_at: null,
    artifact_refs: [],
  };
}
