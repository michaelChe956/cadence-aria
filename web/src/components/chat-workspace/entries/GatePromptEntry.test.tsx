import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
import {
  handleWorkspaceWsMessage,
  type WsServerMessage,
} from "../../../hooks/workspace-ws-message-handler";
import { GatePromptEntry } from "./GatePromptEntry";

function gateEntry(
  actionBlockReason: "terminal_stage" | "phase_mismatch" | null,
  gateIdentity?: string,
  metadata: Record<string, unknown> = {},
): ChatEntry {
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-09-17T00:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      action_block_reason: actionBlockReason,
      ...(gateIdentity ? { gate_identity: gateIdentity } : {}),
      ...metadata,
    },
  };
}

function actions(): CockpitActionFacade {
  return {
    confirm: vi.fn(() => true),
    confirmReview: vi.fn(() => true),
    feedback: vi.fn(() => true),
    terminate: vi.fn(() => true),
    advance: vi.fn(() => true),
    adoptReview: vi.fn(() => true),
    confirmBatch: vi.fn(async () => undefined),
    recoverCompile: vi.fn(async () => undefined),
  };
}


describe("GatePromptEntry actionability", () => {
  installWorkspaceStoreTestHooks();
  it("replaces every gate action with its block reason when the projection is stale", () => {
    const gateActions = actions();

    render(<GatePromptEntry entry={gateEntry("terminal_stage")} actions={gateActions} />);

    expect(screen.getByText("已离开人工确认门")).toBeVisible();
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止此门" })).toBeNull();
    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.feedback).not.toHaveBeenCalled();
    expect(gateActions.terminate).not.toHaveBeenCalled();
  });
  it("locks a rebuilt legacy gate card when the live stage leaves human confirmation", () => {
    const gateActions = actions();
    useWorkspaceStore.getState().setStage("human_confirm");

    render(
      <GatePromptEntry
        entry={gateEntry(null, "stage:human_confirm")}
        actions={gateActions}
      />,
    );
    expect(screen.getByRole("button", { name: "确认当前版本" })).toBeVisible();

    act(() => useWorkspaceStore.getState().setStage("compile_plan"));

    expect(screen.getByText("已离开人工确认门")).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止此门" })).toBeNull();
  });
  it("locks a rebuilt typed gate card when the live stage leaves human confirmation", () => {
    const gateActions = actions();
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 1);

    render(
      <GatePromptEntry
        entry={gateEntry(null, "turn_1")}
        actions={gateActions}
      />,
    );
    expect(screen.getByRole("button", { name: "确认当前版本" })).toBeVisible();

    act(() =>
      handleWorkspaceWsMessage(
        { type: "stage_change", stage: "compile_plan" } as WsServerMessage,
        {
          invalidatedPreStageNodeIds: new Set<string>(),
          scheduleFlush: vi.fn(),
          streamFlushTimeouts: {},
        },
      ),
    );

    expect(screen.getByText("已离开人工确认门")).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止此门" })).toBeNull();
  });

  // F-38：确认卡此前只说「等待人工确认」——确认者不知道在确认什么。有产物版本时
  // 说明区露出「待确认产物：<种类> vN」+「查看产物」入口；无版本则不猜（fail-closed）。
  describe("F-38 pending artifact entry", () => {
    function artifactVersion(versionNo: number, isCurrent: boolean) {
      return {
        version: versionNo,
        generated_by: "pi" as const,
        reviewed_by: null,
        review_verdict: null,
        confirmed_by: null,
        is_current: isCurrent,
        created_at: "2026-09-22T16:02:09Z",
        source_node_id: `timeline_node_00${versionNo}`,
      };
    }

    it("names the pending artifact and opens the artifact view from the gate card", async () => {
      const gateActions = actions();
      const onOpenArtifact = vi.fn();
      const user = userEvent.setup();
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        artifactVersions: [artifactVersion(1, false), artifactVersion(2, true)],
      });

      render(
        <GatePromptEntry
          entry={gateEntry(null)}
          actions={gateActions}
          onOpenArtifact={onOpenArtifact}
        />,
      );

      expect(screen.getByTestId("gate-artifact-context")).toHaveTextContent(
        "待确认产物：Work Item Plan v2",
      );
      await user.click(screen.getByRole("button", { name: "查看产物" }));
      expect(onOpenArtifact).toHaveBeenCalledOnce();
      expect(gateActions.confirm).not.toHaveBeenCalled();
    });

    it("falls back to the latest version when no version is flagged current", () => {
      useWorkspaceStore.setState({
        workspaceType: "story",
        artifactVersions: [artifactVersion(1, false), artifactVersion(2, false)],
      });

      render(
        <GatePromptEntry entry={gateEntry(null)} actions={actions()} onOpenArtifact={vi.fn()} />,
      );

      expect(screen.getByTestId("gate-artifact-context")).toHaveTextContent(
        "待确认产物：Story Spec v2",
      );
    });

    it("renders no artifact entry when the session has no artifact version", () => {
      useWorkspaceStore.setState({ workspaceType: "work_item_plan", artifactVersions: [] });

      render(
        <GatePromptEntry entry={gateEntry(null)} actions={actions()} onOpenArtifact={vi.fn()} />,
      );

      expect(screen.queryByTestId("gate-artifact-context")).toBeNull();
      expect(screen.queryByRole("button", { name: "查看产物" })).toBeNull();
    });

    it("keeps the artifact entry off cards that have no artifact view to open", () => {
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        artifactVersions: [artifactVersion(1, true)],
      });

      render(<GatePromptEntry entry={gateEntry(null)} actions={actions()} />);

      expect(screen.queryByTestId("gate-artifact-context")).toBeNull();
    });

    it("drops the pending artifact line once the gate is resolved", () => {
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        artifactVersions: [artifactVersion(1, true)],
      });

      render(
        <GatePromptEntry
          entry={{ ...gateEntry(null), resolved: true, resolution: "confirm" }}
          actions={actions()}
          onOpenArtifact={vi.fn()}
        />,
      );

      expect(screen.queryByTestId("gate-artifact-context")).toBeNull();
      expect(screen.getByText("已确认")).toBeVisible();
    });
  });

  it("keeps typed gate controls wired to the supplied facade when no block reason exists", async () => {
    const gateActions = actions();
    const user = userEvent.setup();

    render(<GatePromptEntry entry={gateEntry(null)} actions={gateActions} />);

    await user.type(screen.getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(screen.getByRole("button", { name: "提交反馈" }));
    fireEvent.click(screen.getByRole("button", { name: "确认当前版本" }));
    fireEvent.click(screen.getByRole("button", { name: "终止此门" }));
    fireEvent.click(screen.getByRole("button", { name: "确认终止此门" }));

    expect(gateActions.feedback).toHaveBeenCalledWith("请补齐边界");
    expect(gateActions.confirm).toHaveBeenCalledOnce();
    expect(gateActions.terminate).toHaveBeenCalledOnce();
  });

  // F-21（v28 监控 0449）：plan 会话停在 human_confirm 的 context blocker 门
  //（prepare 相位）——actionBlockReason=phase_mismatch 时终止仍必须露出并接
  // facade.terminate（confirm/反馈编辑器维持相位纪律不渲染）。
  it("renders a terminate-only plan gate card for a phase-mismatched context blocker gate (F-21)", async () => {
    const gateActions = actions();
    const user = userEvent.setup();
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "prepare",
      sessionStatus: "waiting_for_human",
      humanGateClosure: null,
    });

    render(
      <GatePromptEntry
        entry={gateEntry("phase_mismatch", "stage:human_confirm")}
        actions={gateActions}
      />,
    );

    expect(screen.getByRole("button", { name: "终止此门" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();

    await user.click(screen.getByRole("button", { name: "终止此门" }));
    expect(gateActions.terminate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "确认终止此门" }));
    expect(gateActions.terminate).toHaveBeenCalledOnce();
    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.feedback).not.toHaveBeenCalled();
  });

  // F-49 A2：门卡身份与当前门投影不一致 ⇒ 该卡不是当前门（已被新一轮取代）。
  // 修前 mismatch 直接回退 persistedReason（= 开门时刻的 action_block_reason，
  // 实测 null）→ 旧卡照常渲染可点按钮，而页面只挂一个动作门面 ⇒ 点旧卡实际作用于
  // 当前门（诊断 §0-B：点旧卡 confirm 使 sendConfirm 被调 1 次）。
  it("locks a gate card whose identity is superseded by the live gate (F-49 A2)", () => {
    const gateActions = actions();
    const store = useWorkspaceStore.getState();
    store.setStage("human_confirm");
    store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

    render(
      <GatePromptEntry
        entry={gateEntry(null, "snapshot:2026-09-24T05:29:45.514Z:native_human_required|2|true||")}
        actions={gateActions}
      />,
    );

    expect(screen.getByText("该人工确认门已关闭")).toBeVisible();
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止此门" })).toBeNull();
    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.feedback).not.toHaveBeenCalled();
  });

  it("locks a gate card once the gate projection disappears (F-49 A2)", () => {
    const gateActions = actions();

    render(<GatePromptEntry entry={gateEntry(null, "turn_1")} actions={gateActions} />);

    expect(screen.getByText("该人工确认门已关闭")).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
  });

  // F-49 A4：提交成功后清空输入并给出成功反馈；此前文本永久留在输入框（用户以
  // 为未提交）。门面拒发（返回 false）时不得清空、不得谎报成功。
  it("clears the feedback input and reports the accepted submission (F-49 A4)", async () => {
    const gateActions = actions();
    const user = userEvent.setup();
    const entry = gateEntry(null);
    useWorkspaceStore.getState().appendChatEntry(entry);

    render(<GatePromptEntry entry={entry} actions={gateActions} />);

    await user.type(screen.getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(screen.getByRole("button", { name: "提交反馈" }));

    expect(gateActions.feedback).toHaveBeenCalledWith("请补齐边界");
    expect(screen.getByLabelText("门禁反馈")).toHaveValue("");
    expect(screen.getByTestId("gate-feedback-submitted")).toHaveTextContent("反馈已提交");
    expect(useWorkspaceStore.getState().chatEntries[0]?.metadata).toMatchObject({
      submitted_feedback: "请补齐边界",
    });
  });

  it("keeps the typed feedback when the gate facade refuses the submission (F-49 A4)", async () => {
    const gateActions = { ...actions(), feedback: vi.fn(() => false) };
    const user = userEvent.setup();

    render(<GatePromptEntry entry={gateEntry(null)} actions={gateActions} />);

    await user.type(screen.getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(screen.getByRole("button", { name: "提交反馈" }));

    expect(screen.getByLabelText("门禁反馈")).toHaveValue("请补齐边界");
    expect(screen.queryByTestId("gate-feedback-submitted")).toBeNull();
  });

  // F-49 B5：旧轮收口后留档卡——只读，带轮次与已提交反馈摘要；动作面整块消失。
  it("renders the archived round note on a superseded card and hides its actions (F-49 B5)", () => {
    const gateActions = actions();
    const archived: ChatEntry = {
      ...gateEntry(null, "snapshot:2026-09-24T05:29:45.514Z|2|true||"),
      resolved: true,
      resolution: "superseded",
      metadata: {
        gate_identity: "snapshot:2026-09-24T05:29:45.514Z|2|true||",
        gate_archive_round: 1,
        gate_archive_note: "第 1 轮已提交反馈：在修复一下 review 审核出来的问题吧",
      },
    };

    render(<GatePromptEntry entry={archived} actions={gateActions} />);

    expect(
      screen.getByText("第 1 轮已提交反馈：在修复一下 review 审核出来的问题吧"),
    ).toBeVisible();
    expect(screen.getByText("已留档")).toBeVisible();
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
    expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止此门" })).toBeNull();
  });

  // F-49 B1/B2/B3：门卡此前只有 4 字 trigger chip，无「为什么需要你」、无「建议确认
  // 还是反馈」，findings 只在「requiresTriage 且 0 条」时被用来提示一句——实测 metadata
  // 带 3 条 advisory 却完全不渲染。文案为 controller 草案（常量在 gate-prompt-copy.ts）。
  describe("F-49 gate guidance", () => {
    const advisoryFindings = [
      { severity: "suggestion", message: "建议补充复杂度说明" },
      { severity: "suggestion", message: "建议统一命名" },
    ];

    it("states why the human is needed for an advisory-only round (B1)", () => {
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            findings: advisoryFindings,
            verdict: "pass",
            review_gate: "user_confirm_allowed",
          })}
          actions={actions()}
        />,
      );

      expect(screen.getByTestId("gate-why")).toHaveTextContent(
        "原因：机械校验 0 error，复评有 2 条建议（不阻断发布）；可直接确认，或提交反馈后再修订。",
      );
    });

    it("switches the reason line to the must-fix copy when a required finding is present (B1)", () => {
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            findings: [...advisoryFindings, { severity: "must_fix", message: "缺少验证命令" }],
            verdict: "revise",
            review_gate: "requires_revision",
          })}
          actions={actions()}
        />,
      );

      expect(screen.getByTestId("gate-why")).toHaveTextContent("原因：有 1 条必须处理项；建议先提交反馈。");
    });

    it("renders no reason line when the round carries no findings (B1 fail-closed)", () => {
      render(<GatePromptEntry entry={gateEntry(null)} actions={actions()} />);

      expect(screen.queryByTestId("gate-why")).toBeNull();
    });

    // F-50 裁决 2：「机械校验 0 error」并入原因行前半句，不再渲染独立的
    // 绿色建议行（gate-advice 退役）。
    it("merges the 0-error advice into the single reason line (F-50)", () => {
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            findings: advisoryFindings,
            verdict: "pass",
            review_gate: "user_confirm_allowed",
          })}
          actions={actions()}
        />,
      );

      expect(screen.getByTestId("gate-why")).toHaveTextContent("机械校验 0 error");
      expect(screen.queryByTestId("gate-advice")).toBeNull();
    });

    it("renders the round findings inside a collapsed list with the verdict-card styles (B3)", () => {
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            findings: [
              { severity: "must_fix", message: "缺少验证命令", required_action: "补充验证命令" },
              { severity: "suggestion", message: "建议补充复杂度说明" },
            ],
            verdict: "revise",
            review_gate: "requires_revision",
          })}
          actions={actions()}
        />,
      );

      const list = screen.getByTestId("gate-findings");
      expect(list.tagName).toBe("DETAILS");
      expect(list).not.toHaveAttribute("open");
      expect(screen.getByText("需要解决")).toBeInTheDocument();
      expect(screen.getByText("高 · 必须修复")).toBeInTheDocument();
      expect(screen.getByText("可选建议")).toBeInTheDocument();
      const rows = screen.getAllByTestId("review-finding");
      expect(rows).toHaveLength(2);
      for (const row of rows) {
        expect(list.contains(row)).toBe(true);
      }
    });

    it("hides the folded findings list once the card is resolved (B3)", () => {
      render(
        <GatePromptEntry
          entry={{
            ...gateEntry(null, undefined, {
              findings: advisoryFindings,
              verdict: "pass",
              review_gate: "user_confirm_allowed",
            }),
            resolved: true,
            resolution: "confirm",
          }}
          actions={actions()}
        />,
      );

      expect(screen.queryByTestId("gate-findings")).toBeNull();
    });
  });

  // F-49 B4：提交反馈后门卡必须报出进程——此前 submitted 后只剩门卡状态悄悄变化，
  // 「author 修订步不可见」（诊断问 3）。文案常量在 gate-prompt-copy.ts；草案的
  // 「第 N/3 轮」序号无可用单调事实源（门每次重开预算重置为默认值，remaining_budget
  // 非单调；turn 计数不在 session_state 内）故不绑序号——需新增事实面才能补，见报告。
  describe("F-49 revision progress copy", () => {
    function turnCard() {
      return gateEntry(null, "turn_1", { turn_id: "turn_1", action_facade: "typed" });
    }

    it("reports the running revision while the typed turn is in flight (B4)", () => {
      const store = useWorkspaceStore.getState();
      store.setStage("human_confirm");
      store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

      render(<GatePromptEntry entry={turnCard()} actions={actions()} />);

      expect(screen.getByTestId("gate-revision-status")).toHaveTextContent(
        "已提交，正在按反馈修订",
      );
    });

    it("reports the finished revision once the turn completes (B4)", () => {
      const store = useWorkspaceStore.getState();
      store.setStage("human_confirm");
      store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);
      store.applyHumanGateTurnCompleted("turn_1", "artifact_9");

      render(<GatePromptEntry entry={turnCard()} actions={actions()} />);

      expect(screen.getByTestId("gate-revision-status")).toHaveTextContent(
        "修订完成，正在复评",
      );
    });

    it("reports no progress line for a gate card without a live turn (B4 fail-closed)", () => {
      const store = useWorkspaceStore.getState();
      store.setStage("human_confirm");
      store.applyHumanGateTurnOpen("turn_other", "cmd_other", 2);

      render(<GatePromptEntry entry={turnCard()} actions={actions()} />);

      expect(screen.queryByTestId("gate-revision-status")).toBeNull();
    });

    it("leaves the failed turn to the failure line instead of a progress note (B4)", () => {
      const store = useWorkspaceStore.getState();
      store.setStage("human_confirm");
      store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);
      store.applyHumanGateTurnFailed("turn_1", "provider_err", "provider 退出");
      // live 路径失败时重铸的门卡携失败元数据（buildGatePromptEntry 取 turn 的
      // failure_class/failure_message）。
      const failedCard = gateEntry(null, "turn_1", {
        turn_id: "turn_1",
        failure_class: "provider_err",
        failure_message: "provider 退出",
      });

      render(<GatePromptEntry entry={failedCard} actions={actions()} />);

      expect(screen.queryByTestId("gate-revision-status")).toBeNull();
      expect(screen.getByTestId("gate-failure")).toHaveTextContent("provider 退出");
    });

    it("gives way to the revision progress line after a successful submit (B4)", async () => {
      const gateActions = actions();
      const user = userEvent.setup();
      const entry = turnCard();
      const store = useWorkspaceStore.getState();
      store.setStage("human_confirm");
      store.appendChatEntry(entry);
      store.applyHumanGateTurnOpen("turn_1", "cmd_1", 2);

      render(<GatePromptEntry entry={entry} actions={gateActions} />);
      expect(screen.getByTestId("gate-revision-status")).toHaveTextContent(
        "已提交，正在按反馈修订",
      );

      await user.type(screen.getByLabelText("门禁反馈"), "请补齐边界");
      await user.click(screen.getByRole("button", { name: "提交反馈" }));

      expect(gateActions.feedback).toHaveBeenCalledWith("请补齐边界");
      // 进程行已含「已提交」语义，不再并列第二条成功反馈。
      expect(screen.queryByTestId("gate-feedback-submitted")).toBeNull();
      expect(screen.getByLabelText("门禁反馈")).toHaveValue("");
    });
  });

  // F-49 B6：advisory findings 一键采纳为反馈草稿——按钮在 findings 列表区顶部，
  // 点击把 advisory（非 must_fix）findings 按模板填入下方反馈输入框（用户可继续
  // 编辑，不自动提交；已有输入时追加防覆盖）。must_fix 不进默认采纳（处理路径不
  // 同）；全部 must_fix 即无 advisory 可采纳，按钮不显示。模板常量在
  // gate-prompt-copy.ts（单测见 gate-prompt-copy.test.ts）。
  describe("F-49 adopt advisory findings", () => {
    const mixedFindings = [
      { severity: "must_fix", message: "缺少验证命令", required_action: "补充验证命令" },
      { severity: "suggestion", message: "复杂度说明", required_action: "补充复杂度说明" },
      { severity: "suggestion", message: "统一命名" },
    ];

    it("fills the feedback input with only the advisory findings on click (B6)", () => {
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            findings: mixedFindings,
            verdict: "revise",
            review_gate: "requires_revision",
          })}
          actions={actions()}
        />,
      );

      const button = screen.getByTestId("gate-adopt-findings");
      expect(button).toHaveTextContent("采纳建议为反馈");
      fireEvent.click(button);

      expect(screen.getByLabelText("门禁反馈")).toHaveValue(
        "按复评建议修订以下内容：复杂度说明：补充复杂度说明；统一命名；其余内容保持不变。",
      );
    });

    it("appends the adopted text without overwriting the typed feedback (B6)", async () => {
      const user = userEvent.setup();
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            findings: mixedFindings.slice(1),
            verdict: "revise",
            review_gate: "requires_revision",
          })}
          actions={actions()}
        />,
      );

      await user.type(screen.getByLabelText("门禁反馈"), "先修标题");
      fireEvent.click(screen.getByTestId("gate-adopt-findings"));

      expect(screen.getByLabelText("门禁反馈")).toHaveValue(
        "先修标题 按复评建议修订以下内容：复杂度说明：补充复杂度说明；统一命名；其余内容保持不变。",
      );
    });

    // F49-B6-fix1：重复点击不重复追加——同一 findings 的采纳文本是确定性的，
    // 草稿已含该段即跳过（防手抖双击把同一串建议拼两遍）。
    it("adopts the advisory findings only once on repeated clicks (F49-B6-fix1)", () => {
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            findings: mixedFindings.slice(1),
            verdict: "revise",
            review_gate: "requires_revision",
          })}
          actions={actions()}
        />,
      );

      const button = screen.getByTestId("gate-adopt-findings");
      fireEvent.click(button);
      fireEvent.click(button);

      expect(screen.getByLabelText("门禁反馈")).toHaveValue(
        "按复评建议修订以下内容：复杂度说明：补充复杂度说明；统一命名；其余内容保持不变。",
      );
    });

    it("hides the adopt button when every finding is must-fix (B6)", () => {
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            findings: [{ severity: "must_fix", message: "缺少验证命令" }],
            verdict: "revise",
            review_gate: "requires_revision",
          })}
          actions={actions()}
        />,
      );

      expect(screen.getByTestId("gate-findings")).toBeInTheDocument();
      expect(screen.queryByTestId("gate-adopt-findings")).toBeNull();
    });

    it("hides the adopt button when no feedback editor is available (B6 fail-closed)", () => {
      render(
        <GatePromptEntry
          entry={gateEntry("phase_mismatch", undefined, {
            findings: mixedFindings.slice(1),
            verdict: "revise",
            review_gate: "requires_revision",
          })}
          actions={actions()}
        />,
      );

      expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
      expect(screen.queryByTestId("gate-adopt-findings")).toBeNull();
    });
  });

  // F-50 §4.1-3/4（第一批布局减负）：门卡固定为「标题→原因→产物→证据→
  // metadata→进度→动作」顺序；trigger 与预算从两个胶囊合并为一条弱化元数据行，
  // 不再与标题组抢主视觉。
  describe("F-50 gate card layout", () => {
    function artifactVersion(versionNo: number, isCurrent: boolean) {
      return {
        version: versionNo,
        generated_by: "pi" as const,
        reviewed_by: null,
        review_verdict: null,
        confirmed_by: null,
        is_current: isCurrent,
        created_at: "2026-09-24T00:00:00Z",
        source_node_id: `timeline_node_00${versionNo}`,
      };
    }

    function layoutEntry(): ChatEntry {
      return gateEntry(null, "turn_layout", {
        turn_id: "turn_layout",
        findings: [
          { severity: "suggestion", message: "建议补充复杂度说明" },
          { severity: "suggestion", message: "建议统一命名" },
        ],
        verdict: "pass",
        review_gate: "user_confirm_allowed",
        gate_trigger: "native_human_required",
        remaining_budget: 3,
      });
    }

    it("trigger 与预算合并为一条弱化元数据行，不再渲染两个胶囊", () => {
      render(<GatePromptEntry entry={layoutEntry()} actions={actions()} />);

      const meta = screen.getByTestId("gate-meta");
      expect(meta).toHaveTextContent("触发：引擎判定需人工");
      expect(meta).toHaveTextContent("剩余修复轮次 3");
      expect(meta.className).not.toContain("aria-chip");
      expect(screen.queryByTestId("gate-trigger-label")).toBeNull();
      expect(screen.queryByTestId("gate-budget")).toBeNull();
    });

    it("门卡顺序：原因 → 产物 → 证据 → 元数据 → 进度", () => {
      const store = useWorkspaceStore.getState();
      store.setStage("human_confirm");
      store.applyHumanGateTurnOpen("turn_layout", "cmd_layout", 3);
      store.applyHumanGateTurnCompleted("turn_layout", "artifact_9");
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        artifactVersions: [artifactVersion(6, true)],
      });

      render(
        <GatePromptEntry
          entry={layoutEntry()}
          actions={actions()}
          onOpenArtifact={vi.fn()}
        />,
      );

      const why = screen.getByTestId("gate-why");
      const artifact = screen.getByTestId("gate-artifact-context");
      const findings = screen.getByTestId("gate-findings");
      const meta = screen.getByTestId("gate-meta");
      const progress = screen.getByTestId("gate-revision-status");
      expect(
        why.compareDocumentPosition(artifact) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      expect(
        artifact.compareDocumentPosition(findings) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      expect(
        findings.compareDocumentPosition(meta) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      expect(
        meta.compareDocumentPosition(progress) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    });
  });

  // F-50 裁决 1/3/4/9（第二批语义文案）：单标题制、反馈与确认并行、终止按
  // 门级作用域命名、颜色契约（门禁=琥珀，不染 system 红）。
  describe("F-50 gate card semantics", () => {
    it("单标题制：默认门卡只有一个「需要人工确认」标题", () => {
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            verdict: "needs_human",
            review_gate: "requires_revision",
          })}
          actions={actions()}
        />,
      );

      expect(screen.getByText("需要人工确认")).toBeVisible();
      expect(screen.queryByText("可确认当前版本")).toBeNull();
      expect(screen.queryByText("人工确认")).toBeNull();
    });

    it("triage intent 门保留「需要判断 reviewer 意图」并配 intent 原因行", () => {
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            verdict: "needs_human",
            review_gate: "user_triage_required",
          })}
          actions={actions()}
        />,
      );

      expect(screen.getByText("需要判断 reviewer 意图")).toBeVisible();
      // F-50 fix round 落地核查：triage 形态同样走 GATE_CARD_CLASS——中性白底+
      // 琥珀左线（两分支共用同一 panelClassName，无 triage 专属旧面板残留）。
      const card = screen.getByTestId("gate-prompt-entry");
      expect(card.className).toContain("bg-white");
      expect(card.className).toContain("border-l-4");
      expect(card.className).toContain("border-l-amber-500/60");
      expect(card.className).not.toContain("bg-[var(--aria-gate-open-bg)]");
      expect(screen.getByTestId("gate-why")).toHaveTextContent(
        "原因：评审结果无法自动取舍；请选择确认当前版本或反馈修改。",
      );
    });

    it("entry.content 与标题同义时不渲染（单标题不留同义正文）", () => {
      render(
        <GatePromptEntry
          entry={{
            ...gateEntry(null),
            content: "需要人工确认",
            metadata: { summary: "等待人工确认" },
          }}
          actions={actions()}
        />,
      );

      // 标题保留一份，同义的正文/摘要全部让位。
      expect(screen.getAllByText("需要人工确认")).toHaveLength(1);
      expect(screen.queryByText("等待人工确认")).toBeNull();
    });

    it("独立事实的 content 与 summary 保留渲染；summary 与 content 同义只留一份", () => {
      render(
        <GatePromptEntry
          entry={{
            ...gateEntry(null),
            content: "复评发现边界场景缺口",
            metadata: { summary: "复评发现边界场景缺口" },
          }}
          actions={actions()}
        />,
      );

      expect(screen.getAllByText("复评发现边界场景缺口")).toHaveLength(1);
    });

    it("反馈与确认并行：帮助文案标明反馈可选、确认无需填写", () => {
      render(<GatePromptEntry entry={gateEntry(null)} actions={actions()} />);

      expect(screen.getByTestId("gate-feedback-hint")).toHaveTextContent(
        "如需调整，请在反馈框填写修改意见；确认当前版本无需填写。",
      );
    });

    it("修订进行中隐藏静态反馈指导（F-50 §2.3-5）", () => {
      const store = useWorkspaceStore.getState();
      store.setStage("human_confirm");
      store.applyHumanGateTurnOpen("turn_hint", "cmd_hint", 2);

      render(
        <GatePromptEntry
          entry={gateEntry(null, "turn_hint", { turn_id: "turn_hint" })}
          actions={actions()}
        />,
      );

      expect(screen.getByTestId("gate-revision-status")).toHaveTextContent(
        "已提交，正在按反馈修订",
      );
      expect(screen.queryByTestId("gate-feedback-hint")).toBeNull();
    });

    it("视觉 v2：中性卡底+琥珀左线+chip，按钮权重阶梯实心>描边>ghost（f50-ui-visual-spec-v2 §1/§2）", () => {
      render(<GatePromptEntry entry={gateEntry(null)} actions={actions()} />);

      const card = screen.getByTestId("gate-prompt-entry");
      // 门卡=中性底，琥珀只上左线（4px）/chip/图标——不再整卡琥珀底（琥珀压琥珀根因）。
      expect(card.className).toContain("bg-white");
      expect(card.className).toContain("border-l-4");
      expect(card.className).toContain("border-l-amber-500/60");
      expect(card.className).not.toContain("bg-[var(--aria-gate-open-bg)]");
      expect(card.className).not.toContain("bg-red-50");
      // 标题行：text-base + slate 正文主色（不染 system 红的旧约束保留）。
      const title = screen.getByText("需要人工确认");
      expect(title.className).toContain("text-slate-900");
      expect(title.className).not.toContain("text-red-500");
      // chip：琥珀只出现在标签。
      expect(screen.getByText("需人工").className).toContain("text-amber-700");
      // 主操作=实心 emerald（白底描边旧形态退役，权重淹没根因）。
      const confirm = screen.getByRole("button", { name: "确认当前版本" });
      expect(confirm.className).toContain("bg-emerald-600");
      expect(confirm.className).not.toContain("border-emerald-200");
      // Ghost 终止=无底透明红字，与主/次操作拉开权重阶梯。
      const terminate = screen.getByRole("button", { name: "终止此门" });
      expect(terminate.className).toContain("text-red-600");
      expect(terminate.className).not.toContain("bg-white");
    });

    it("视觉 v2：查看产物=次操作描边钮 min-h-9，findings 折叠去原生三角（§2/§4）", () => {
      useWorkspaceStore.setState({
        workspaceType: "work_item_plan",
        artifactVersions: [
          {
            version: 2,
            generated_by: "pi",
            reviewed_by: null,
            review_verdict: null,
            confirmed_by: null,
            is_current: true,
            created_at: "2026-09-22T16:02:09Z",
            source_node_id: "timeline_node_002",
          },
        ],
      });
      render(
        <GatePromptEntry
          entry={gateEntry(null, undefined, {
            findings: [{ severity: "suggestion", message: "建议补充复杂度说明" }],
            verdict: "pass",
            review_gate: "user_confirm_allowed",
          })}
          actions={actions()}
          onOpenArtifact={vi.fn()}
        />,
      );

      const openArtifact = screen.getByTestId("gate-artifact-open");
      expect(openArtifact.className).toContain("border-slate-300");
      expect(openArtifact.className).toContain("text-slate-700");
      expect(openArtifact.className).toContain("min-h-9");
      // findings 折叠：原生 ▶ marker 隐藏，chevron-right 由 open 态旋转。
      const findings = screen.getByTestId("gate-findings");
      const summary = findings.querySelector("summary");
      expect(summary?.className).toContain("list-none");
      const chevron = summary?.querySelector("svg.lucide-chevron-right");
      expect(chevron?.getAttribute("class")).toContain("group-open:rotate-90");
      expect(chevron).not.toBeNull();
    });
  });
});
