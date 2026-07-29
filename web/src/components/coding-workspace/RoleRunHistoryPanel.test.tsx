import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  CodingRoleRun,
  CodingRoleRunEventPreview,
  CodingTimelineNode,
} from "../../api/types";
import { RoleRunHistoryPanel } from "./RoleRunHistoryPanel";

describe("RoleRunHistoryPanel", () => {
  it("renders compact run summaries and expands details on demand", () => {
    render(
      <RoleRunHistoryPanel
        roleRuns={[
          roleRun({
            id: "coding_role_run_0001",
            role: "code_reviewer",
            stage: "code_review",
            run_no: 1,
            status: "superseded",
            trigger: "initial",
            node_id: "coding_node_0003",
            superseded_by_run_id: "coding_role_run_0002",
            reason_code: "review_payload_parse_error",
            raw_provider_output_refs: ["provider-raw/code-review/review_0001.txt"],
            event_summary: {
              event_count: 3,
              last_event_at: "2026-06-13T00:00:03Z",
              last_event_type: "execution_event",
              last_event_title: "Task update",
              last_event_status: "running",
              terminal_event_type: "timeout",
              terminal_reason: "review_timeout",
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
                detail: "Reviewing changes",
                truncated: true,
                artifact_ref:
                  "artifacts/role-run-events/coding_role_run_0001/0003_output.txt",
              },
            ],
          }),
          roleRun({
            id: "coding_role_run_0002",
            role: "code_reviewer",
            stage: "code_review",
            run_no: 2,
            status: "completed",
            trigger: "retry_review",
            node_id: "coding_node_0004",
            artifact_refs: ["provider-raw/code-review/review_0002.json"],
          }),
        ]}
        timelineNodes={[
          node("coding_node_0003", "代码审查"),
          node("coding_node_0004", "代码审查重跑"),
        ]}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveTextContent("角色运行历史");
    expect(panel).toHaveTextContent("Code Reviewer #1");
    expect(panel).toHaveTextContent("已被替代");
    expect(panel).toHaveTextContent("initial");
    expect(panel).toHaveTextContent("review_payload_parse_error");
    expect(panel).toHaveTextContent("3 events");
    expect(panel).toHaveTextContent("Task update");
    expect(panel).toHaveTextContent("running");
    expect(panel).toHaveTextContent("Code Reviewer #2");
    expect(panel).toHaveTextContent("已完成");
    expect(panel).toHaveTextContent("retry_review");
    expect(panel).toHaveTextContent("代码审查重跑");
    expect(panel).not.toHaveTextContent("review_timeout");
    expect(panel).not.toHaveTextContent("No tasks found");
    expect(panel).not.toHaveTextContent(
      "artifacts/role-run-events/coding_role_run_0001/0003_output.txt",
    );
    expect(panel).not.toHaveTextContent("provider-raw/code-review/review_0001.txt");

    fireEvent.click(screen.getByRole("button", { name: /Code Reviewer #1/ }));

    expect(panel).toHaveTextContent("review_timeout");
    expect(panel).toHaveTextContent("#2");
    expect(panel).toHaveTextContent("#3");
    expect(panel).toHaveTextContent("No tasks found");
    expect(panel).toHaveTextContent(
      "artifacts/role-run-events/coding_role_run_0001/0003_output.txt",
    );
    expect(panel).toHaveTextContent("provider-raw/code-review/review_0001.txt");
  });

  it("selects the linked timeline node", () => {
    const onSelectNode = vi.fn();
    render(
      <RoleRunHistoryPanel
        roleRuns={[roleRun({ node_id: "coding_node_0005" })]}
        timelineNodes={[node("coding_node_0005", "Code Reviewer")]}
        selectedNodeId={null}
        onSelectNode={onSelectNode}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Code Reviewer #1/ }));

    expect(onSelectNode).toHaveBeenCalledWith("coding_node_0005");
  });

  it("shows coder role runs with elapsed duration", () => {
    render(
      <RoleRunHistoryPanel
        roleRuns={[
          roleRun({
            id: "coding_role_run_0003",
            role: "coder",
            stage: "coding",
            run_no: 1,
            status: "completed",
            trigger: "initial",
            node_id: "coding_node_0001",
            started_at: "2026-06-13T00:00:00Z",
            completed_at: "2026-06-13T00:02:03Z",
          }),
        ]}
        timelineNodes={[node("coding_node_0001", "代码编写")]}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveTextContent("Coder #1");
    expect(panel).toHaveTextContent("已完成");
    expect(panel).toHaveTextContent("耗时 2分03秒");
  });

  it("labels internal reviewer runs as GroupFinalReview", () => {
    render(
      <RoleRunHistoryPanel
        roleRuns={[
          roleRun({
            role: "internal_reviewer",
            stage: "internal_pr_review",
            run_no: 1,
            status: "completed",
            trigger: "initial",
            node_id: "coding_node_group_final_review",
          }),
        ]}
        timelineNodes={[node("coding_node_group_final_review", "GroupFinalReview")]}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveTextContent("GroupFinalReview #1");
    expect(panel).not.toHaveTextContent("Internal Reviewer");
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
        timelineNodes={[node("coding_node_0005", "系统处理")]}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Code Reviewer #1/ }));

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
    stage: "code_review",
    role: "code_reviewer",
    run_no: 1,
    status: "blocked",
    trigger: "retry_review",
    node_id: "coding_node_0005",
    started_at: "2026-06-13T00:00:00Z",
    completed_at: null,
    supersedes_run_id: null,
    superseded_by_run_id: null,
    reason_code: "code_review_blocked",
    raw_provider_output_refs: [],
    artifact_refs: [],
    ...overrides,
  };
}

function node(id: string, title: string): CodingTimelineNode {
  return {
    id,
    attempt_id: "coding_attempt_0001",
    stage: "code_review",
    title,
    status: "blocked",
    agent_role: "reviewer",
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
