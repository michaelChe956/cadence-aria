import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceStore } from "../../state/workspace-ws-store";
import { ChatInputBar } from "./ChatInputBar";

describe("ChatInputBar", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
  });

  it("supports prepare context submission and optimistic insertion", () => {
    const onSendContextNote = vi.fn();
    const onStartGeneration = vi.fn();
    const onAbort = vi.fn();
    const onSendHumanDecision = vi.fn();

    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={onSendContextNote}
        onStartGeneration={onStartGeneration}
        onSendHumanDecision={onSendHumanDecision}
        onAbort={onAbort}
      />,
    );

    fireEvent.change(screen.getByRole("textbox"), { target: { value: "补充上下文" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    expect(onSendContextNote).toHaveBeenCalledWith("补充上下文");
    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "context_note",
        role: "user",
        content: "补充上下文",
      }),
    ]);
    expect(screen.getByRole("button", { name: "开始生成" })).toBeInTheDocument();
  });

  it("disables input while running and exposes abort only", () => {
    render(
      <ChatInputBar
        stage="running"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onSendHumanDecision={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    expect(screen.getByRole("textbox")).toBeDisabled();
    expect(screen.queryByRole("button", { name: "开始生成" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "中止" })).toBeInTheDocument();
  });

  it("submits human confirm feedback with optimistic insertion", () => {
    const onSendHumanDecision = vi.fn();
    useWorkspaceStore.getState().appendChatEntry({
      id: "gate-1",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      timestamp: "2026-05-21T10:00:00Z",
    });

    render(
      <ChatInputBar
        stage="human_confirm"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onSendHumanDecision={onSendHumanDecision}
        onAbort={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByRole("textbox"), { target: { value: "补充失败路径" } });
    fireEvent.click(screen.getByRole("button", { name: "发送修改意见" }));

    expect(onSendHumanDecision).toHaveBeenCalledWith("补充失败路径");
    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "gate-1",
        resolved: true,
        resolution: "request-change",
      }),
      expect.objectContaining({
        type: "human_decision",
        role: "user",
        content: "补充失败路径",
      }),
    ]);
  });
});
