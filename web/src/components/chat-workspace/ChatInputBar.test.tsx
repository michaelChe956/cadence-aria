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

    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={onSendContextNote}
        onStartGeneration={onStartGeneration}
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
        onAbort={vi.fn()}
      />,
    );

    expect(screen.getByRole("textbox")).toBeDisabled();
    expect(screen.getByRole("button", { name: "中止" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "发送" })).toBeNull();
  });

  it("hides start generation while an interrupted run is recoverable", () => {
    render(
      <ChatInputBar
        stage="prepare_context"
        hideStartGeneration={true}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: "开始生成" })).toBeNull();
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
        onAuthorDecision={onAuthorDecision}
        onAbort={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByRole("textbox"), { target: { value: "补充回滚策略" } });
    fireEvent.click(screen.getByRole("button", { name: "发送反馈" }));

    expect(onAuthorDecision).toHaveBeenCalledWith("revise", "补充回滚策略");
  });

  // spec-workbench-canvas-experience T4：预填能力改为 ref 暴露（供面板采纳按钮调用），
  // 覆盖式写入，重复调用不拼接。
  it("exposes a covering prefill handle and input focus callback", () => {
    const onInputFocus = vi.fn();
    const ref = createRef<ChatInputBarHandle>();
    render(
      <ChatInputBar
        ref={ref}
        stage="prepare_context"
        onInputFocus={onInputFocus}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    const input = screen.getByRole("textbox");
    fireEvent.focus(input);
    expect(onInputFocus).toHaveBeenCalledTimes(1);

    act(() => {
      ref.current?.prefill("按以下 review 意见修订：\n\n- 补充边界条件");
      ref.current?.prefill("第二次覆盖式预填");
    });
    expect(input).toHaveValue("第二次覆盖式预填");
  });

  // L1（REQ-RET-02）：human_confirm 决策输入发送面（发送修改意见=request-change）随
  // legacy 决策退役删除——该阶段输入只读、无发送按钮（决策走 typed 门动作面）。
  // 原「submits human confirm feedback with optimistic insertion」
  // 「prevents a host-blocked human confirmation from sending a decision」两测随之退役。

  it("renders the human confirm stage read-only without a decision send path", () => {
    render(
      <ChatInputBar
        stage="human_confirm"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    const input = screen.getByRole("textbox");
    expect(input).toBeDisabled();
    expect(input).toHaveAttribute("placeholder", "人工确认阶段（决策走门禁操作）");
    expect(screen.queryByRole("button", { name: "发送修改意见" })).toBeNull();
    expect(screen.queryByRole("button", { name: "发送" })).toBeNull();
  });

  it("keeps the author confirm input usable when the host provides a decision callback", () => {
    render(
      <ChatInputBar
        stage="author_confirm"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAuthorDecision={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    expect(screen.getByRole("textbox")).toBeEnabled();
  });
});
