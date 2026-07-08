import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry, WorkspaceContentRef } from "../../state/chat-entries";
import { InlineEventRow } from "./InlineEventRow";

describe("InlineEventRow", () => {
  it("renders collapsed by default and toggles command output details", () => {
    render(
      <InlineEventRow
        entry={{
          id: "event-1",
          type: "execution_event",
          role: "system",
          content: "读取认证模块",
          timestamp: "2026-05-26T10:00:00Z",
          metadata: {
            command: "sed -n '1,120p' src/auth.rs",
            output: "ok",
          },
        }}
      />,
    );

    expect(screen.getByText("读取认证模块")).toBeInTheDocument();
    expect(screen.queryByText("sed -n '1,120p' src/auth.rs")).not.toBeInTheDocument();
    expect(screen.queryByText("ok")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /读取认证模块/ }));

    expect(screen.getByText("sed -n '1,120p' src/auth.rs")).toBeInTheDocument();
    expect(screen.getByText("ok")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /读取认证模块/ }));

    expect(screen.queryByText("sed -n '1,120p' src/auth.rs")).not.toBeInTheDocument();
    expect(screen.queryByText("ok")).not.toBeInTheDocument();
  });

  it("decodes and formats html entity escaped event detail, command and output", () => {
    const { container } = render(
      <InlineEventRow
        entry={{
          id: "event-entity",
          type: "execution_event",
          role: "system",
          content: "校验 Draft",
          timestamp: "2026-05-26T10:00:00Z",
          metadata: {
            detail: "当前 &quot;Draft&quot; 输出",
            command: "echo &quot;safe&quot;",
            output: "{&quot;required_gates&quot;:[&quot;cmd_check&quot;]}",
          },
        }}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /校验 Draft/ }));

    expect(screen.getByText('当前 "Draft" 输出')).toBeInTheDocument();
    expect(screen.getByText('echo "safe"')).toBeInTheDocument();
    expect(container.textContent).toContain('"required_gates": [');
    expect(container.textContent).not.toContain("&quot;");
  });

  it("按需加载 execution output 并复用缓存", async () => {
    let resolveContent: (value: string) => void = () => undefined;
    const loadContent = vi.fn().mockImplementation(
      () =>
        new Promise<string>((resolve) => {
          resolveContent = resolve;
        }),
    );
    const contentCache: Record<string, string> = {};
    const entry: ChatEntry = {
      id: "event-1",
      type: "execution_event",
      role: "system",
      content: "运行测试",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: {
        command: "pnpm test",
      },
      content_ref: { kind: "execution_output", nodeId: "node-1", eventId: "event-1" },
    };

    const { rerender } = render(
      <InlineEventRow
        entry={entry}
        sessionId="session-1"
        contentCache={contentCache}
        loadContent={loadContent}
        onCacheContent={(key, value) => {
          contentCache[key] = value;
        }}
      />,
    );

    expect(loadContent).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: /运行测试/ }));

    expect(screen.getByText("加载输出中...")).toBeInTheDocument();
    await waitFor(() => expect(loadContent).toHaveBeenCalledTimes(1));
    expect(loadContent).toHaveBeenCalledWith("session-1", {
      kind: "execution_output",
      nodeId: "node-1",
      eventId: "event-1",
    });
    resolveContent("完整执行输出\nline 2");
    expect(await screen.findByText(/完整执行输出/)).toBeInTheDocument();
    expect(screen.getByText(/line 2/)).toBeInTheDocument();
    expect(contentCache["execution_output:node-1:event-1"]).toBe("完整执行输出\nline 2");

    fireEvent.click(screen.getByRole("button", { name: /运行测试/ }));
    expect(screen.queryByText(/完整执行输出/)).not.toBeInTheDocument();

    rerender(
      <InlineEventRow
        entry={entry}
        sessionId="session-1"
        contentCache={contentCache}
        loadContent={loadContent}
        onCacheContent={(key, value) => {
          contentCache[key] = value;
        }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /运行测试/ }));

    expect(screen.getByText(/完整执行输出/)).toBeInTheDocument();
    expect(loadContent).toHaveBeenCalledTimes(1);
  });

  it("按需加载 provider prompt 并复用缓存", async () => {
    const loadContent = vi.fn().mockResolvedValue("完整提示词 large-prompt-0\n".repeat(2));
    const contentCache: Record<string, string> = {};
    const entry: ChatEntry = {
      id: "event-prompt",
      type: "execution_event",
      role: "system",
      content: "Story 生成 · Provider Prompt · 120 KB",
      timestamp: "2026-05-26T10:00:00Z",
      content_ref: { kind: "provider_prompt", nodeId: "node-1" },
    };

    render(
      <InlineEventRow
        entry={entry}
        sessionId="session-1"
        contentCache={contentCache}
        loadContent={loadContent}
        onCacheContent={(key, value) => {
          contentCache[key] = value;
        }}
      />,
    );

    expect(screen.getByText("PROMPT")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Provider Prompt/ }));

    expect(screen.getByText("加载 Prompt 中...")).toBeInTheDocument();
    expect(await screen.findByText(/完整提示词 large-prompt-0/)).toBeInTheDocument();
    expect(loadContent).toHaveBeenCalledWith("session-1", {
      kind: "provider_prompt",
      nodeId: "node-1",
    });
    expect(contentCache["provider_prompt:node-1"]).toContain("完整提示词 large-prompt-0");
  });

  it("按需加载 execution output 失败时显示错误", async () => {
    let rejectContent: (error: Error) => void = () => undefined;
    const loadContent = vi.fn().mockImplementation(
      () =>
        new Promise<string>((_resolve, reject) => {
          rejectContent = reject;
        }),
    );

    render(
      <InlineEventRow
        entry={{
          id: "event-1",
          type: "execution_event",
          role: "system",
          content: "运行测试",
          timestamp: "2026-05-26T10:00:00Z",
          content_ref: { kind: "execution_output", nodeId: "node-1", eventId: "event-1" },
        }}
        sessionId="session-1"
        contentCache={{}}
        loadContent={loadContent}
        onCacheContent={() => undefined}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /运行测试/ }));

    expect(screen.getByText("加载输出中...")).toBeInTheDocument();
    rejectContent(new Error("加载输出失败：500"));
    expect(await screen.findByText("加载输出失败：500")).toBeInTheDocument();
  });

  it("父组件在请求 pending 时重渲染不会对同一 cache key 重复加载", async () => {
    const loadContent = vi.fn(
      (_sessionId: string, _ref: WorkspaceContentRef) =>
        new Promise<string>(() => {
          // 保持 pending，覆盖父组件重渲染期间的重复请求风险。
        }),
    );
    const entry: ChatEntry = {
      id: "event-1",
      type: "execution_event",
      role: "system",
      content: "运行测试",
      timestamp: "2026-05-26T10:00:00Z",
      content_ref: { kind: "execution_output", nodeId: "node-1", eventId: "event-1" },
    };

    function Wrapper() {
      const [tick, setTick] = useState(0);
      return (
        <div>
          <button type="button" onClick={() => setTick((current) => current + 1)}>
            rerender {tick}
          </button>
          <InlineEventRow
            entry={entry}
            sessionId="session-1"
            contentCache={{}}
            loadContent={(currentSessionId, ref) => loadContent(currentSessionId, ref)}
            onCacheContent={() => undefined}
          />
        </div>
      );
    }

    render(<Wrapper />);

    fireEvent.click(screen.getByRole("button", { name: /运行测试/ }));
    await waitFor(() => expect(loadContent).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: /rerender/ }));

    expect(screen.getByText("加载输出中...")).toBeInTheDocument();
    expect(loadContent).toHaveBeenCalledTimes(1);
  });

  it("折叠后快速重展开会复用 pending 请求并显示原请求结果", async () => {
    let resolveContent: (value: string) => void = () => undefined;
    const loadContent = vi.fn(
      (_sessionId: string, _ref: WorkspaceContentRef) =>
        new Promise<string>((resolve) => {
          resolveContent = resolve;
        }),
    );
    const contentCache: Record<string, string> = {};
    const entry: ChatEntry = {
      id: "event-1",
      type: "execution_event",
      role: "system",
      content: "运行测试",
      timestamp: "2026-05-26T10:00:00Z",
      content_ref: { kind: "execution_output", nodeId: "node-1", eventId: "event-1" },
    };

    render(
      <InlineEventRow
        entry={entry}
        sessionId="session-1"
        contentCache={contentCache}
        loadContent={loadContent}
        onCacheContent={(key, value) => {
          contentCache[key] = value;
        }}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /运行测试/ }));
    await waitFor(() => expect(loadContent).toHaveBeenCalledTimes(1));
    expect(screen.getByText("加载输出中...")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /运行测试/ }));
    expect(screen.queryByText("加载输出中...")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /运行测试/ }));
    expect(screen.getByText("加载输出中...")).toBeInTheDocument();
    expect(loadContent).toHaveBeenCalledTimes(1);

    resolveContent("重展开后显示的输出");

    expect(await screen.findByText("重展开后显示的输出")).toBeInTheDocument();
    expect(screen.queryByText("加载输出中...")).not.toBeInTheDocument();
    expect(contentCache["execution_output:node-1:event-1"]).toBe("重展开后显示的输出");
    expect(loadContent).toHaveBeenCalledTimes(1);
  });
});
