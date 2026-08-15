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

  it("hides start generation while an interrupted run is recoverable", () => {
    render(
      <ChatInputBar
        stage="prepare_context"
        hideStartGeneration={true}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onSendHumanDecision={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: "开始生成" })).not.toBeInTheDocument();
  });

  it("shows three author confirm actions with a usable feedback input", () => {
    const onAuthorDecision = vi.fn();

    render(
      <ChatInputBar
        stage="author_confirm"
        reviewerEnabled={true}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onSendHumanDecision={vi.fn()}
        onAuthorDecision={onAuthorDecision}
        onAbort={vi.fn()}
      />,
    );

    const feedbackInput = screen.getByPlaceholderText(/输入修改意见/);
    expect(feedbackInput).toBeEnabled();
    expect(screen.getByRole("button", { name: "发送反馈" })).toBeDisabled();
    expect(
      screen.queryByRole("button", { name: /重新编写/ }),
    ).not.toBeInTheDocument();

    fireEvent.change(feedbackInput, { target: { value: "补充回滚策略" } });
    fireEvent.click(screen.getByRole("button", { name: "发送反馈" }));

    expect(onAuthorDecision).toHaveBeenCalledWith("revise", "补充回滚策略");
    expect(
      (screen.getByPlaceholderText(/输入修改意见/) as HTMLTextAreaElement).value,
    ).toBe("");

    fireEvent.click(screen.getByRole("button", { name: "确认并送审" }));
    fireEvent.click(screen.getByRole("button", { name: "确认定稿" }));

    expect(onAuthorDecision).toHaveBeenNthCalledWith(2, "accept_with_review");
    expect(onAuthorDecision).toHaveBeenNthCalledWith(3, "accept_finalize");
  });

  it("highlights finalize by default when review is disabled", () => {
    render(
      <ChatInputBar
        stage="author_confirm"
        reviewerEnabled={false}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onSendHumanDecision={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("button", { name: "确认并送审" }).className,
    ).not.toContain("aria-primary");
    expect(
      screen.getByRole("button", { name: "确认并送审" }),
    ).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "确认定稿" }).className,
    ).toContain("aria-primary");
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

    expect(onSendHumanDecision).toHaveBeenCalledWith({
      description: "补充失败路径",
      source: "human",
    });
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
