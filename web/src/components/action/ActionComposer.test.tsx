import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ActionComposer } from "./ActionComposer";

describe("ActionComposer", () => {
  it("shows codex-like prompt editor and sends confirmed prompt", async () => {
    const onConfirm = vi.fn();
    render(
      <ActionComposer
        pendingStep={{
          node_id: "N16",
          provider_type: "codex",
          runtime_role: "executor",
          adapter_role: "executor",
          prompt: "实现函数",
          input_summary: { worktask_id: "work_wt_001" },
          canonical_input_refs: ["plan_projection_task_0001_0001"],
          context_files: ["openspec/changes/aria-fibonacci-square/tasks.md"],
          output_schema: "schema://aria/artifacts/coding_report/v1",
          allowed_write_scope: ["src/", "tests/"],
          forbidden_actions: ["修改 cadence/project-rules"],
          verification_commands: ["node --test"],
          checkpoint_id: "ckpt_0001",
        }}
        onConfirm={onConfirm}
        onRollback={() => undefined}
        onStop={() => undefined}
        running={false}
      />,
    );

    expect(screen.getByText(/plan_projection_task_0001_0001/)).toBeInTheDocument();
    expect(screen.getByText(/openspec\/changes\/aria-fibonacci-square\/tasks.md/)).toBeInTheDocument();
    expect(screen.getByText(/修改 cadence\/project-rules/)).toBeInTheDocument();
    expect(screen.getByText(/node --test/)).toBeInTheDocument();
    const textarea = screen.getByLabelText("Provider prompt");
    await userEvent.clear(textarea);
    await userEvent.type(textarea, "确认后的 prompt");
    await userEvent.selectOptions(screen.getByLabelText("Policy override"), "manual-all");
    await userEvent.click(screen.getByRole("button", { name: "确认执行" }));

    expect(onConfirm).toHaveBeenCalledWith({
      checkpoint_id: "ckpt_0001",
      prompt: "确认后的 prompt",
      policy_override: "manual-all",
    });
  });

  it("calls stop when a provider run is active", async () => {
    const onStop = vi.fn();
    render(
      <ActionComposer
        pendingStep={{
          node_id: "N16",
          provider_type: "codex",
          runtime_role: "executor",
          adapter_role: "executor",
          prompt: "实现函数",
          input_summary: {},
          canonical_input_refs: [],
          context_files: [],
          output_schema: "schema://aria/artifacts/coding_report/v1",
          allowed_write_scope: ["src/"],
          forbidden_actions: [],
          verification_commands: [],
          checkpoint_id: "ckpt_0001",
        }}
        onConfirm={() => undefined}
        onRollback={() => undefined}
        onStop={onStop}
        running
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "停止" }));
    expect(onStop).toHaveBeenCalled();
  });
});
