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
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止" })).toBeNull();
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
    expect(screen.getByRole("button", { name: "确认产物" })).toBeVisible();

    act(() => useWorkspaceStore.getState().setStage("compile_plan"));

    expect(screen.getByText("已离开人工确认门")).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止" })).toBeNull();
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
    expect(screen.getByRole("button", { name: "确认产物" })).toBeVisible();

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
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止" })).toBeNull();
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
    fireEvent.click(screen.getByRole("button", { name: "确认产物" }));
    fireEvent.click(screen.getByRole("button", { name: "终止" }));
    fireEvent.click(screen.getByRole("button", { name: "确认终止" }));

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

    expect(screen.getByRole("button", { name: "终止" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();

    await user.click(screen.getByRole("button", { name: "终止" }));
    expect(gateActions.terminate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "确认终止" }));
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
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止" })).toBeNull();
    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.feedback).not.toHaveBeenCalled();
  });

  it("locks a gate card once the gate projection disappears (F-49 A2)", () => {
    const gateActions = actions();

    render(<GatePromptEntry entry={gateEntry(null, "turn_1")} actions={gateActions} />);

    expect(screen.getByText("该人工确认门已关闭")).toBeVisible();
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
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
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止" })).toBeNull();
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
        "复评仍有 2 条 findings（均为建议级，不阻断发布）；引擎不做自动取舍，由你确认采纳或反馈修改。",
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

      expect(screen.getByTestId("gate-why")).toHaveTextContent("存在 1 条必须处理项，建议先提交反馈");
    });

    it("renders no reason line when the round carries no findings (B1 fail-closed)", () => {
      render(<GatePromptEntry entry={gateEntry(null)} actions={actions()} />);

      expect(screen.queryByTestId("gate-why")).toBeNull();
    });

    it("advises a direct confirm on advisory-only rounds (B2)", () => {
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

      expect(screen.getByTestId("gate-advice")).toHaveTextContent(
        "机械校验 0 error——可直接确认；如需采纳建议请提交反馈",
      );
    });

    it("omits the advice line when the gate cannot be confirmed (B2 fail-closed)", () => {
      render(
        <GatePromptEntry
          entry={gateEntry("phase_mismatch", "stage:human_confirm", {
            findings: advisoryFindings,
            verdict: "pass",
            review_gate: "user_confirm_allowed",
          })}
          actions={actions()}
        />,
      );

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
});
