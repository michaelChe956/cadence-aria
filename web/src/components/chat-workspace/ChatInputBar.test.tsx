import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { providerHealthSnapshot } from "../../state/provider-availability-test-fixtures";
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

  // 3.6 滚动体系收敛：textarea 不再以 rows=3 固定高度制造小内滚区——
  // 高度随内容自适应（80–240px），封顶后超出部分才在框内滚动（可达性保留）。
  // jsdom 无布局，用实例属性模拟 scrollHeight 驱动自适应逻辑。
  it("grows the textarea with content up to a cap and keeps manual resize", () => {
    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    const input = screen.getByRole("textbox") as HTMLTextAreaElement;
    // resize 契约不破坏（约束：不得移除输入框 resize）
    expect(input.className).toContain("resize-y");
    // 封顶（max-h-60）
    let mockScrollHeight = 500;
    Object.defineProperty(input, "scrollHeight", {
      get: () => mockScrollHeight,
      configurable: true,
    });
    fireEvent.change(input, { target: { value: "x".repeat(600) } });
    expect(input.style.height).toBe("240px");
    // 随内容收缩
    mockScrollHeight = 120;
    fireEvent.change(input, { target: { value: "中等内容" } });
    expect(input.style.height).toBe("122px");
    // 下限（min-h-20）
    mockScrollHeight = 40;
    fireEvent.change(input, { target: { value: "短" } });
    expect(input.style.height).toBe("80px");
  });

  it("resets the textarea height to the minimum after sending", () => {
    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    const input = screen.getByRole("textbox") as HTMLTextAreaElement;
    let mockScrollHeight = 300;
    Object.defineProperty(input, "scrollHeight", {
      get: () => mockScrollHeight,
      configurable: true,
    });
    fireEvent.change(input, { target: { value: "多行反馈意见" } });
    expect(input.style.height).toBe("240px");

    mockScrollHeight = 60;
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    expect(input).toHaveValue("");
    expect(input.style.height).toBe("80px");
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

  // v38 复验 #2/#3（恢复 C3 前原意）：story/design AuthorConfirm 门的反馈修订
  // 发送通道——宿主传入 onSendRevisionFeedback 时输入+发送可用，提交走修订
  // 回调（不再走 context_note），乐观条目落对话流；未传入（如 WorkItemPlan
  // 门或宿主不接线）时保持只读呈现、无发送钮。
  it("sends revision feedback at author confirm when the host wires the channel", () => {
    const onSendRevisionFeedback = vi.fn(() => true);
    render(
      <ChatInputBar
        stage="author_confirm"
        onSendRevisionFeedback={onSendRevisionFeedback}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    const input = screen.getByRole("textbox");
    expect(input).toBeEnabled();
    fireEvent.change(input, {
      target: { value: "按以下 review 意见修订：\n\n第二段缺少冲突" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发送反馈" }));

    expect(onSendRevisionFeedback).toHaveBeenCalledWith(
      "按以下 review 意见修订：\n\n第二段缺少冲突",
    );
    expect(input).toHaveValue("");
    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "context_note",
        role: "user",
        content: "按以下 review 意见修订：\n\n第二段缺少冲突",
      }),
    ]);
  });

  it("keeps the author confirm input read-only without a host revision channel", () => {
    render(
      <ChatInputBar
        stage="author_confirm"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    expect(screen.getByRole("textbox")).toBeEnabled();
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "不该有发送通道" },
    });
    expect(screen.queryByRole("button", { name: "发送反馈" })).toBeNull();
    expect(screen.queryByRole("button", { name: "发送" })).toBeNull();
  });

  it("blocks revision feedback send on empty input or failed dispatch", () => {
    const onSendRevisionFeedback = vi.fn(() => false);
    render(
      <ChatInputBar
        stage="author_confirm"
        onSendRevisionFeedback={onSendRevisionFeedback}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
      />,
    );

    const input = screen.getByRole("textbox");
    // 空输入：发送反馈不可点。
    expect(screen.getByRole("button", { name: "发送反馈" })).toBeDisabled();
    // 发送失败（宿主返回 false）：反馈不丢——输入保留、不落乐观条目。
    fireEvent.change(input, { target: { value: "断线时不能丢反馈" } });
    fireEvent.click(screen.getByRole("button", { name: "发送反馈" }));
    expect(onSendRevisionFeedback).toHaveBeenCalledWith("断线时不能丢反馈");
    expect(input).toHaveValue("断线时不能丢反馈");
    expect(useWorkspaceStore.getState().chatEntries).toEqual([]);
  });

  // F-28（v33 复验 3）+ F-50 裁决 6：STALE_DRIVER_LEASE 就地错误面中文主显
  //（连接租约已失效）+ mono 错误码副行 + 中文正文，英文原文进折叠详情。
  it("renders the stale-lease notice with Chinese lead, mono code, folded original (F-28/F-50)", () => {
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

    const alert = screen.getByTestId("hard-error-notice");
    expect(alert).toHaveTextContent("连接租约已失效");
    const code = within(alert).getByTestId("hard-error-code");
    expect(code).toHaveTextContent("STALE_DRIVER_LEASE");
    expect(code.className).toContain("aria-mono");
    expect(alert).toHaveTextContent("本连接已失去写入租约，当前操作未提交。");
    expect(
      within(screen.getByTestId("hard-error-details")).getByText(
        /driver connection no longer holds the lease/,
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "开始生成" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "重新接管" }));
    expect(onRetakeLease).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "确认重新接管" }));
    expect(onRetakeLease).toHaveBeenCalledTimes(1);
  });

  // F-50 裁决 7：抽屉打开时完整错误动作面归抽屉，页级缩为引用面（无重接管钮）。
  it("shrinks the page-level notice to a drawer reference while the drawer owns the actions (F-50)", () => {
    render(
      <ChatInputBar
        stage="prepare_context"
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        onAbort={vi.fn()}
        hardErrorNotice={{
          code: "STALE_DRIVER_LEASE",
          message: "driver connection no longer holds the lease",
          onRetakeLease: null,
          referenceNote: "处理入口在待处理抽屉",
        }}
      />,
    );

    const alert = screen.getByTestId("hard-error-notice");
    expect(alert).toHaveTextContent("处理入口在待处理抽屉");
    expect(alert).not.toHaveTextContent(/建议刷新页面/);
    expect(screen.queryByRole("button", { name: "重新接管" })).toBeNull();
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

  // REQ-PPS-03：开始生成前显示将要使用的实际 provider；不可用即 fail-closed 禁用+原因，
  // 不让「先用服务端默认、再由页面纠正」的隐式窗口出现。
  describe("generation provider visibility", () => {
    afterEach(() => {
      useProviderAvailabilityStore.getState().reset();
    });

    function renderPrepareBar(
      stage = "prepare_context",
      author: "claude_code" | "codex" = "claude_code",
    ) {
      useWorkspaceStore.setState({
        providers: { author, reviewer: "codex" },
      });
      return render(
        <ChatInputBar
          stage={stage}
          onSendContextNote={vi.fn()}
          onStartGeneration={vi.fn()}
          onAbort={vi.fn()}
        />,
      );
    }

    it("shows the effective author provider and its availability beside 开始生成", () => {
      useProviderAvailabilityStore.setState({
        loadStatus: "loaded",
        snapshot: providerHealthSnapshot({ claude_code: true, codex: true }),
      });

      // 会话侧选定的 author provider 就是展示与启动使用的那一个。
      renderPrepareBar("prepare_context", "codex");

      const status = screen.getByTestId("start-generation-provider");
      expect(status).toHaveTextContent("Codex");
      expect(status).toHaveTextContent("可用");
      expect(screen.getByTestId("start-generation")).toBeEnabled();
      expect(screen.queryByTestId("start-generation-blocked-hint")).toBeNull();
    });

    it("blocks 开始生成 and shows the reason when the author provider is unavailable", () => {
      useProviderAvailabilityStore.setState({
        loadStatus: "loaded",
        snapshot: providerHealthSnapshot({ codex: true, claude_code: false }),
      });

      renderPrepareBar();

      expect(screen.getByTestId("start-generation")).toBeDisabled();
      expect(screen.getByTestId("start-generation-provider")).toHaveTextContent(
        "Claude Code",
      );
      expect(screen.getByTestId("start-generation-provider")).toHaveTextContent(
        "不可用",
      );
      expect(
        screen.getByTestId("start-generation-blocked-hint"),
      ).toHaveTextContent("Claude Code 未安装");
    });

    it("leaves later stages to their regular input state", () => {
      useProviderAvailabilityStore.setState({
        loadStatus: "loaded",
        snapshot: providerHealthSnapshot({ claude_code: false }),
      });

      renderPrepareBar("running");

      expect(screen.queryByTestId("start-generation")).toBeNull();
      expect(screen.queryByTestId("start-generation-provider")).toBeNull();
      expect(screen.queryByTestId("start-generation-blocked-hint")).toBeNull();
      expect(screen.getByRole("button", { name: "中止" })).toBeInTheDocument();
      expect(screen.getByRole("textbox")).toBeDisabled();
    });
  });
});
