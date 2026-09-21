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

  // 退役留档（T5/REQ-RET-02）：`keeps only the feedback send action at author confirm` 驱动已删除的 legacy 决策发送面，
  // 随消息族退役（wp5-attribution-table.md）；T1 矩阵 legacy 回归留档在案。

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

  // F-28（v33 复验 3）：STALE_DRIVER_LEASE 就地错误面在生成动作区直出——
  // 传入 notice 即渲染「连接租约已过期」文案与二次确认重接管；宿主不传则不渲染。
  it("renders the stale-lease notice with a confirm-twice retake beside the action buttons (F-28)", () => {
    const onRetakeLease = vi.fn();
    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
        hardErrorNotice={{
          code: "STALE_DRIVER_LEASE",
          message: "driver connection no longer holds the lease",
          onRetakeLease,
        }}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(/连接租约已过期/);
    expect(alert).toHaveTextContent("STALE_DRIVER_LEASE");
    expect(screen.getByRole("button", { name: "开始生成" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "重新接管" }));
    expect(onRetakeLease).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "确认重新接管" }));
    expect(onRetakeLease).toHaveBeenCalledTimes(1);
  });

  // F-28 二轮（v34 复验「失败态点开始生成零反馈」）：就地面泛化 hard_error
  // 全族——长开 tab 断线重连后拒收码可能是 OBSERVER_WRITE_REJECTED（本连接以
  // observer 身份重连，写操作被拒），同样给「重新接管」二次确认（sendHello
  // role=driver 可夺回租约）。
  it("offers the confirm-twice retake for the observer-write-rejected notice (F-28 R2)", () => {
    const onRetakeLease = vi.fn();
    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
        hardErrorNotice={{
          code: "OBSERVER_WRITE_REJECTED",
          message: "observer connection cannot send write message start_generation",
          onRetakeLease,
        }}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(/观察者/);
    expect(alert).toHaveTextContent("OBSERVER_WRITE_REJECTED");
    expect(alert).not.toHaveTextContent(/连接租约已过期/);

    fireEvent.click(screen.getByRole("button", { name: "重新接管" }));
    expect(onRetakeLease).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "确认重新接管" }));
    expect(onRetakeLease).toHaveBeenCalledTimes(1);
  });

  // 其余未知 hard_error 码：错误码直出 + 建议刷新文案，不给重接管钮——
  // 无凭据表明 sendHello 能恢复，重接管入口只留给 lease 拒收两码。
  it("shows the code and refresh advice without a retake for unknown hard-error codes (F-28 R2)", () => {
    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
        hardErrorNotice={{
          code: "SESSION_ALREADY_CONFIRMED",
          message: "会话已确认（终态），不能重新开始生成",
          onRetakeLease: null,
        }}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("SESSION_ALREADY_CONFIRMED");
    expect(alert).toHaveTextContent(/刷新/);
    expect(alert).not.toHaveTextContent(/连接租约已过期/);
    expect(screen.queryByRole("button", { name: "重新接管" })).toBeNull();
  });

  // F-30 共存：终态禁用提示与 hard_error 就地面同屏——禁用态吞不掉错误面
  //（长开 tab 终态残留 + 拒收错误叠加的真实形态：提示优先且错误面仍可见）。
  it("keeps the hard-error notice visible beside the terminal-session disabled hint (F-28 R2 × F-30)", () => {
    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
        startGenerationDisabled={true}
        startGenerationDisabledHint="会话已确认（终态）——重新生成请走修订流程或新建会话"
        hardErrorNotice={{
          code: "OBSERVER_WRITE_REJECTED",
          message: "observer connection cannot send write message start_generation",
          onRetakeLease: vi.fn(),
        }}
      />,
    );

    expect(screen.getByTestId("start-generation")).toBeDisabled();
    expect(screen.getByTestId("start-generation-blocked-hint")).toHaveTextContent(
      /终态/,
    );
    expect(screen.getByRole("alert")).toHaveTextContent(/OBSERVER_WRITE_REJECTED/);
  });

  it("renders no hard-error notice without the host-provided notice (F-28)", () => {
    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText(/连接租约已过期/)).toBeNull();
  });

  // 退役留档（T5/REQ-RET-02）：`keeps the author confirm input usable when the host provides a decision callback` 驱动已删除的 legacy 决策发送面，
  // 随消息族退役（wp5-attribution-table.md）；T1 矩阵 legacy 回归留档在案。
});
