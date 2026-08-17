import { act, fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceStore } from "../../state/workspace-ws-store";
import { ChatInputBar, type ChatInputBarHandle } from "./ChatInputBar";

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

  // spec-workbench-canvas-experience T4：确认并送审/确认定稿/采纳 Review 意见
  // 已迁移至 ArtifactReviewPanel，此处仅保留反馈发送。
  it("keeps only the feedback send action at author confirm", () => {
    const onAuthorDecision = vi.fn();

    render(
      <ChatInputBar
        stage="author_confirm"
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
      screen.queryByRole("button", { name: "确认并送审" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "确认定稿" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "采纳 Review 意见" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /重新编写/ }),
    ).not.toBeInTheDocument();

    fireEvent.change(feedbackInput, { target: { value: "补充回滚策略" } });
    fireEvent.click(screen.getByRole("button", { name: "发送反馈" }));

    expect(onAuthorDecision).toHaveBeenCalledWith("revise", "补充回滚策略");
    expect(
      (screen.getByPlaceholderText(/输入修改意见/) as HTMLTextAreaElement).value,
    ).toBe("");
  });

  // spec-workbench-canvas-experience T4：预填能力改为 ref 暴露（供面板采纳按钮调用），
  // 覆盖式写入，重复调用不拼接。
  it("exposes a covering prefill handle and input focus callback", () => {
    const onInputFocus = vi.fn();
    const ref = createRef<ChatInputBarHandle>();

    render(
      <ChatInputBar
        ref={ref}
        stage="author_confirm"
        onInputFocus={onInputFocus}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onSendHumanDecision={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    act(() => {
      ref.current?.prefill("按以下 review 意见修订：\n\n发现 3 个问题");
    });
    const feedbackInput = screen.getByPlaceholderText(
      /输入修改意见/,
    ) as HTMLTextAreaElement;
    expect(feedbackInput.value).toBe("按以下 review 意见修订：\n\n发现 3 个问题");

    act(() => {
      ref.current?.prefill("第二次预填");
    });
    expect(feedbackInput.value).toBe("第二次预填");

    fireEvent.focus(feedbackInput);
    expect(onInputFocus).toHaveBeenCalledTimes(1);
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
