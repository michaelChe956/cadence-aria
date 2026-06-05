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
