import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { TopStatusBar } from "./TopStatusBar";

describe("TopStatusBar", () => {
  it("renders blocked_by_gate breakdown instead of a single failed label", () => {
    render(
      <TopStatusBar
        projection={{
          workspace_root: "/tmp/aria-workspace",
          active_task_id: "task_0001",
          overview: {
            status: "blocked_by_gate",
            change_id: "aria-fibonacci-square",
            current_node: "N18",
            current_worktask: "work_wt_001",
            policy_preset: "manual-write",
            provider_mode: "fake",
            e2e_overall: "blocked_by_gate",
            business_code: "generated",
            unit_tests: "passed",
            coverage_gate: "passed",
            archive_worktask: "failed",
            root_cause: "cadence/ write scope missing",
          },
          git_summary: { branch: "main", head: "abc1234", dirty: true },
          sse_connected: true,
          running_state: "blocked",
        }}
      />,
    );

    expect(screen.getByText(/Business code: generated/)).toBeInTheDocument();
    expect(screen.getByText(/Unit tests: passed/)).toBeInTheDocument();
    expect(screen.getByText(/Archive worktask: failed/)).toBeInTheDocument();
    expect(screen.getByText(/cadence\/ write scope missing/)).toBeInTheDocument();
  });

  it("exposes connection and running state as accessible status pills", () => {
    render(
      <TopStatusBar
        projection={{
          active_task_id: "task_0001",
          overview: {
            status: "running",
            change_id: "aria-fibonacci-square",
            current_node: "N16",
          },
          git_summary: { branch: "main", head: "abc1234", dirty: false },
          sse_connected: true,
          running_state: "running",
        }}
      />,
    );

    expect(screen.getByLabelText("SSE connected")).toHaveTextContent("connected");
    expect(screen.getByLabelText("运行状态 running")).toHaveTextContent("running");
    expect(screen.getByText("task_0001")).toBeInTheDocument();
  });
});
