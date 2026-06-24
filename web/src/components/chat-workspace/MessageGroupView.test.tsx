import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../state/chat-entries";
import { MessageGroupView } from "./MessageGroupView";
import type { MessageGroup } from "./message-grouping";

describe("MessageGroupView", () => {
  it("renders primary stream, inline event and permission interrupt in one author group", () => {
    const onPermissionResponse = vi.fn();

    render(
      <MessageGroupView
        group={{
          id: "group-1",
          nodeId: "node-1",
          role: "author",
          primaryEntry: makeEntry("stream-1", "provider_stream", "author", "# Story\n\n内容"),
          inlineEvents: [
            makeEntry("event-1", "execution_event", "system", "读取文件", {
              command: "cat src/lib.rs",
            }),
          ],
          interruptEntries: [
            makeEntry("permission-1", "permission_request", "system", "shell · cargo test", {
              request_id: "permission-1",
              tool_name: "shell",
              description: "cargo test",
            }),
          ],
        }}
        onPermissionResponse={onPermissionResponse}
      />,
    );

    expect(screen.getByText("作者")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Story" })).toBeInTheDocument();
    expect(screen.getByText("内容")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /读取文件/ }));
    expect(screen.getByText("cat src/lib.rs")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "允许" }));
    expect(onPermissionResponse).toHaveBeenCalledWith(
      expect.objectContaining({ id: "permission-1" }),
      true,
    );
  });

  it("uses reviewer title for reviewer groups", () => {
    render(
      <MessageGroupView
        group={{
          id: "group-reviewer",
          nodeId: "node-reviewer",
          role: "reviewer",
          primaryEntry: makeEntry("stream-reviewer", "provider_stream", "reviewer", "审核通过", {
            provider: "codex",
          }),
          inlineEvents: [],
          interruptEntries: [],
        }}
      />,
    );

    expect(screen.getByText("审核者 · Codex")).toBeInTheDocument();
    expect(screen.getByText("审核通过")).toBeInTheDocument();
  });

  it("renders grouped primary stream markdown semantics in the message bubble", () => {
    render(
      <MessageGroupView
        group={{
          id: "group-markdown",
          nodeId: "node-markdown",
          role: "author",
          primaryEntry: makeEntry(
            "stream-markdown",
            "provider_stream",
            "author",
            "## 结论\n\n- 审核 **通过**\n- 使用 `pnpm test` 验证",
          ),
          inlineEvents: [],
          interruptEntries: [],
        }}
      />,
    );

    expect(screen.getByRole("heading", { name: "结论" })).toBeInTheDocument();
    expect(screen.getByRole("list")).toBeInTheDocument();
    expect(screen.getByText("通过").tagName).toBe("STRONG");
    expect(screen.getByText("pnpm test").tagName).toBe("CODE");
  });

  it("formats grouped tester TestPlan JSON streams before rendering markdown", () => {
    const escapedPlan = JSON.stringify({
      summary: "针对本次 diff 生成 Provider 依赖自检验证计划",
      assumptions: ["当前阶段仅生成 TestPlan，不直接执行命令。"],
      steps: [
        {
          id: "step_001_rules_context",
          title: "读取仓库验证规则",
          required: true,
          risk_level: "low",
          evidence_expectation: "读取到规则文件内容。",
          related_requirements: ["REQ-provider-gate"],
          related_work_item_tasks: ["TASK-007"],
        },
      ],
    }).replaceAll('"', "&quot;");

    const { container } = render(
      <MessageGroupView
        group={{
          id: "group-tester-plan",
          nodeId: "coding_node_0003",
          role: "tester",
          primaryEntry: makeEntry("tester-plan", "provider_stream", "tester", escapedPlan, {
            provider: "codex",
            phase: "plan_tests",
          }),
          inlineEvents: [],
          interruptEntries: [],
        }}
      />,
    );

    expect(screen.getByText("Tester · Codex")).toBeInTheDocument();
    expect(screen.getByText("Tester 测试计划")).toBeInTheDocument();
    expect(screen.getByText(/针对本次 diff 生成 Provider 依赖自检验证计划/)).toBeInTheDocument();
    expect(screen.getByText(/step_001_rules_context/)).toBeInTheDocument();
    expect(screen.getByText(/TASK-007/)).toBeInTheDocument();
    expect(container.textContent).not.toContain("&quot;");
    expect(container.textContent).not.toContain('"summary"');
  });

  it("shows reviewer tool calls even when no stream text has arrived", () => {
    render(
      <MessageGroupView
        group={{
          id: "group-reviewer-tools",
          nodeId: "node-reviewer",
          role: "reviewer",
          inlineEvents: [
            makeEntry("event-1", "execution_event", "reviewer", "git diff --stat", {
              agent: "codex",
              command: "git diff --stat",
              output: "changed files",
            }),
          ],
          interruptEntries: [],
        }}
      />,
    );

    expect(screen.getByText("审核者 · Codex")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /git diff --stat/ }));
    expect(screen.getByText("changed files")).toBeInTheDocument();
  });

  it.each([
    ["tester", "Tester · Fake · Run #2"],
    ["analyst", "Analyst · Fake · Run #3"],
    ["code_reviewer", "Code Reviewer · Fake · Run #4"],
    ["internal_reviewer", "Internal Reviewer · Fake · Run #5"],
  ] as const)("shows run number in %s group title", (role, expectedTitle) => {
    const runNo = Number(expectedTitle.match(/#(\d+)/)?.[1]);
    render(
      <MessageGroupView
        group={{
          id: `group-${role}`,
          nodeId: "coding_node_0001",
          role,
          primaryEntry: makeEntry(
            `entry-${role}`,
            "provider_stream",
            role,
            "Readable provider output",
            {
              provider: "fake",
              role_run_id: `coding_role_run_${runNo}`,
              run_no: runNo,
            },
          ),
          inlineEvents: [],
          interruptEntries: [],
        }}
      />,
    );

    expect(screen.getByText(expectedTitle)).toBeInTheDocument();
  });

  it("marks automatic retry groups and exposes the previous attempt error", () => {
    render(
      <MessageGroupView
        group={{
          id: "group-retry",
          nodeId: "timeline_node_007",
          role: "author",
          primaryEntry: makeEntry(
            "retry-stream",
            "provider_stream",
            "author",
            "修正后的完整 outline",
            {
              provider: "codex",
              retry: {
                retry_of_node_id: "timeline_node_006",
                retry_attempt: 2,
                retry_reason: "outline_structured_output_parse_error",
                retry_error: {
                  code: "outline_structured_output_parse_error",
                  message:
                    "Provider did not return a valid WorkItemPlan Outline structured output: invalid structured output json",
                },
              },
            },
          ),
          inlineEvents: [],
          interruptEntries: [],
        }}
      />,
    );

    expect(screen.getByText("作者 · Codex · 自动重跑 #2")).toBeInTheDocument();
    expect(screen.getByText("自动重跑 #2")).toBeInTheDocument();
    fireEvent.click(screen.getByText("上一轮错误"));
    expect(
      screen.getByText(/Provider did not return a valid WorkItemPlan Outline structured output/),
    ).toBeInTheDocument();
  });
});

function makeEntry(
  id: string,
  type: ChatEntry["type"],
  role: ChatEntry["role"],
  content: string,
  metadata?: Record<string, unknown>,
): ChatEntry {
  return {
    id,
    type,
    role,
    content,
    timestamp: "2026-05-26T10:00:00Z",
    node_id: "node-1",
    metadata,
  };
}
