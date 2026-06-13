import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  CodingRoleRun,
  CodingRoleRunEventPreview,
  CodingTimelineNode,
} from "../../api/types";
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
            event_summary: {
              event_count: 3,
              last_event_at: "2026-06-13T00:00:03Z",
              last_event_type: "execution_event",
              last_event_title: "Task update",
              last_event_status: "running",
              terminal_event_type: "timeout",
              terminal_reason: "plan_tests_timeout",
            },
            recent_events: [
              {
                sequence: 2,
                event_type: "text_delta",
                created_at: "2026-06-13T00:00:02Z",
                title: "text_delta",
                status: null,
                detail: "No tasks found",
                truncated: false,
                artifact_ref: null,
              },
              {
                sequence: 3,
                event_type: "execution_event",
                created_at: "2026-06-13T00:00:03Z",
                title: "Task update",
                status: "running",
                detail: "Planning tests",
                truncated: true,
                artifact_ref:
                  "artifacts/role-run-events/coding_role_run_0001/0003_output.txt",
              },
            ],
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
    expect(panel).toHaveTextContent("3 events");
    expect(panel).toHaveTextContent("Task update");
    expect(panel).toHaveTextContent("running");
    expect(panel).toHaveTextContent("plan_tests_timeout");
    expect(panel).toHaveTextContent("#2");
    expect(panel).toHaveTextContent("#3");
    expect(panel).toHaveTextContent("No tasks found");
    expect(panel).toHaveTextContent(
      "artifacts/role-run-events/coding_role_run_0001/0003_output.txt",
    );
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

  it("renders only the latest three recent events", () => {
    render(
      <RoleRunHistoryPanel
        roleRuns={[
          roleRun({
            recent_events: [
              recentEvent(1, "Dropped oldest event"),
              recentEvent(2, "Visible event 2"),
              recentEvent(3, "Visible event 3"),
              recentEvent(4, "Visible event 4"),
            ],
          }),
        ]}
        timelineNodes={[node("coding_node_0005", "Analyst 路由决策")]}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).not.toHaveTextContent("Dropped oldest event");
    expect(panel).toHaveTextContent("Visible event 2");
    expect(panel).toHaveTextContent("Visible event 3");
    expect(panel).toHaveTextContent("Visible event 4");
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

function recentEvent(sequence: number, detail: string): CodingRoleRunEventPreview {
  return {
    sequence,
    event_type: "execution_event",
    created_at: `2026-06-13T00:00:0${sequence}Z`,
    title: detail,
    status: null,
    detail,
    truncated: false,
    artifact_ref: null,
  };
}
