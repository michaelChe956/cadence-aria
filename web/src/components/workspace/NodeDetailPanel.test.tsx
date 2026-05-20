import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { NodeDetail, TimelineNode } from "../../api/types";
import { NodeDetailPanel } from "./NodeDetailPanel";

describe("NodeDetailPanel", () => {
  it("renders 5 node-level tabs", () => {
    render(<NodeDetailPanel node={node()} detail={detail()} artifactVersions={[]} />);

    expect(screen.getByTestId("node-detail-panel")).toBeInTheDocument();
    expect(screen.getByTestId("tab-overview")).toHaveTextContent("概览");
    expect(screen.getByTestId("tab-streaming")).toHaveTextContent("流式输出");
    expect(screen.getByTestId("tab-execution")).toHaveTextContent("执行事件");
    expect(screen.getByTestId("tab-permission")).toHaveTextContent("权限");
    expect(screen.getByTestId("tab-artifact")).toHaveTextContent("Artifact");
  });

  it("switches to streaming, execution, permission, and artifact tabs", () => {
    render(
      <NodeDetailPanel
        node={node()}
        detail={{
          ...detail(),
          execution_events: [
            {
              event_id: "event-1",
              kind: "command",
              status: "completed",
              title: "运行测试",
              command: "pnpm test",
            },
          ],
          permission_events: [
            {
              request_id: "perm-1",
              request: { tool_name: "bash" },
              response: { approved: true },
              ts: "2026-05-20T14:31:00Z",
            },
          ],
          artifact_ref: { artifact_id: "node-1", version: 2 },
        }}
        artifactVersions={[
          {
            version: 2,
            markdown: "# Artifact 内容",
            generated_by: "claude_code",
            created_at: "2026-05-20T14:35:00Z",
            source_node_id: "node-1",
          },
        ]}
      />,
    );

    fireEvent.click(screen.getByTestId("tab-streaming"));
    expect(screen.getByTestId("streaming-content")).toHaveTextContent("输出内容");

    fireEvent.click(screen.getByTestId("tab-execution"));
    expect(screen.getByText("运行测试")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("tab-permission"));
    expect(screen.getByText("perm-1")).toBeInTheDocument();
    expect(screen.getByText("已批准")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("tab-artifact"));
    expect(screen.getByText("# Artifact 内容")).toBeInTheDocument();
  });

  it("renders permission event response statuses including timeout", () => {
    render(
      <NodeDetailPanel
        node={node()}
        detail={detail({
          permission_events: [
            {
              request_id: "perm-pending",
              request: { tool_name: "bash" },
              response: null,
              ts: "2026-05-20T14:31:00Z",
            },
            {
              request_id: "perm-approved",
              request: { tool_name: "bash" },
              response: { approved: true },
              ts: "2026-05-20T14:32:00Z",
            },
            {
              request_id: "perm-denied",
              request: { tool_name: "bash" },
              response: { approved: false },
              ts: "2026-05-20T14:33:00Z",
            },
            {
              request_id: "perm-timeout",
              request: { tool_name: "bash" },
              response: { status: "timeout" },
              ts: "2026-05-20T14:34:00Z",
            },
          ],
        })}
        artifactVersions={[]}
      />,
    );

    fireEvent.click(screen.getByTestId("tab-permission"));

    expect(screen.getByText("perm-pending")).toBeInTheDocument();
    expect(screen.getByText("待应答")).toBeInTheDocument();
    expect(screen.getByText("已批准")).toBeInTheDocument();
    expect(screen.getByText("已拒绝")).toBeInTheDocument();
    expect(screen.getByText("超时")).toBeInTheDocument();
  });
});

function node(overrides?: Partial<TimelineNode>): TimelineNode {
  return {
    node_id: "node-1",
    node_type: "author_run",
    stage: "running",
    status: "completed",
    title: "生成",
    started_at: "2026-05-20T14:30:00Z",
    provider_config_snapshot: {
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    },
    ...overrides,
  };
}

function detail(overrides?: Partial<NodeDetail>): NodeDetail {
  return {
    node_id: "node-1",
    session_id: "sess-1",
    node_type: "author_run",
    status: "completed",
    agent_role: "author",
    provider: { name: "claude_code", model: "opus-4-7" },
    messages: [],
    streaming_content: "输出内容",
    execution_events: [],
    permission_events: [],
    verdict: null,
    artifact_ref: null,
    is_revision: false,
    base_artifact_ref: null,
    started_at: "2026-05-20T14:30:00Z",
    ended_at: null,
    ...overrides,
  };
}
