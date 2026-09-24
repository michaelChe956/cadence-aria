import { Check, ChevronRight, FileText } from "lucide-react";
import { useState } from "react";
import type { ChatEntry } from "../../../state/chat-entries";
import {
  gateActionBlockCopy,
  selectGateProjection,
  type GateActionBlockReason,
  type GateProjection,
  GATE_TRIGGER_LABELS,
} from "../../../state/workspace-cockpit-projection";
import {
  GATE_ADOPT_FINDINGS_BUTTON_LABEL,
  GATE_ARCHIVE_BADGE_LABEL,
  GATE_FEEDBACK_OPTIONAL_HINT,
  GATE_FEEDBACK_SUBMITTED_NOTE,
  GATE_REVISION_REVIEWING_NOTE,
  GATE_REVISION_RUNNING_NOTE,
  GATE_TERMINATE_BUTTON_LABEL,
  GATE_TERMINATE_CONFIRM_LABEL,
  GATE_TRIAGE_WHY_COPY,
  gateAdoptFindingsFeedback,
  gateBatchConfirmTitle,
  gateBatchConfirmWhyCopy,
  gateDistanceToPass,
  gateFindingsToggleLabel,
  gateWhyAdvisoryCopy,
  gateWhyRequiredCopy,
  isGateTitleSynonymousCopy,
} from "../../../state/gate-prompt-copy";
import type { WorkspaceWsState } from "../../../state/workspace-ws-store-types";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import type { WorkItemPlanHumanGateSnapshot } from "../../../api/types";
import { WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND } from "../../../state/workspace-chat-rebuild";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { ConfirmTwiceButton } from "../cockpit/ConfirmTwiceButton";
import { GateFeedbackEditor } from "../cockpit/GateFeedbackEditor";
import { ChatEntryContainer } from "../ChatEntryContainer";
import {
  BTN_GHOST_CLASS,
  BTN_PRIMARY_CLASS,
  BTN_SECONDARY_CLASS,
  DISCLOSURE_CHEVRON_CLASS,
  DISCLOSURE_SUMMARY_CLASS,
  GATE_CARD_CLASS,
  GATE_CHIP_CLASS,
  GATE_DIVIDER_CLASS,
  GATE_META_TEXT_CLASS,
  GATE_NESTED_BLOCK_CLASS,
  GATE_SECONDARY_TEXT_CLASS,
  GATE_TITLE_CLASS,
} from "../gate-visual-tokens";
import { isBlockingFinding, ReviewFindingGroups, reviewFindingsFromEntry } from "../finding-list";
import {
  gateFindingsCrossRoundDelta,
  gateFindingsDeltaCopy,
} from "../../../state/gate-prompt-copy";

/** F-38：门卡产物行的种类名（与生命周期卡片同一套称呼）。 */
const GATE_ARTIFACT_LABELS: Record<string, string> = {
  work_item_plan: "Work Item Plan",
  story: "Story Spec",
  design: "Design Spec",
};

export function GatePromptEntry({
  entry,
  actions,
  onOpenArtifact,
}: {
  entry: ChatEntry;
  actions?: CockpitActionFacade;
  /** F-38：切到产物视图（cockpit 产物审核页签）；缺省即不渲染产物入口（legacy 页）。 */
  onOpenArtifact?: () => void;
}) {
  const [feedback, setFeedback] = useState("");
  const [feedbackSubmitted, setFeedbackSubmitted] = useState(false);
  const recordGateFeedbackSubmission = useWorkspaceStore(
    (state) => state.recordGateFeedbackSubmission,
  );
  const summary = summaryFromEntry(entry);
  const verdict = verdictFromEntry(entry);
  const reviewGate = reviewGateFromEntry(entry);
  const findings = reviewFindingsFromEntry(entry);
  // C2（REQ-HGC-03 场景 1）：阻断口径以 effective class（class_hint 优先，
  // severity 保底）为准——severity=suggestion 而 class_hint=repairable 时
  // 原因行不得再写「不阻断发布」。
  const requiredFindings = findings.filter((finding) => isBlockingFinding(finding));
  const requiresTriage = reviewGate === "user_triage_required";
  // F-38：确认者必须知道在确认什么——门卡说明区带出待确认产物版本与入口。
  // 没有产物版本即不渲染（fail-closed 不猜）；版本取 is_current，缺省退最新一轮。
  const workspaceType = useWorkspaceStore((state) => state.workspaceType);
  const pendingArtifactVersion = useWorkspaceStore((state) => {
    const versions = state.artifactVersions ?? [];
    if (versions.length === 0) {
      return null;
    }
    return (
      versions.find((version) => version.is_current === true) ??
      versions.reduce((latest, version) =>
        version.version > latest.version ? version : latest,
      )
    );
  });
  // F-50 裁决 1/3：反馈与确认并行，确认按钮统一「确认当前版本」——与原因行的
  // 「可直接确认」和帮助文案同一称呼。
  const confirmLabel = "确认当前版本";
  // L1（REQ-RET-02）：request-change 按钮随 legacy 决策发送面删除；findings 仅作呈现。
  const isResolved = entry.resolved === true;
  const gateTrigger = gateTriggerFromEntry(entry);
  const remainingBudget = remainingBudgetFromEntry(entry);
  const failureMessage = failureMessageFromEntry(entry);
  const inlineError = inlineErrorFromEntry(entry);
  const isContextBlockerGate =
    gateKindFromEntry(entry) === WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND;
  const actionFacade =
    (entry.metadata as Record<string, unknown> | undefined)?.action_facade;
  const typedGateAwaitingCommand =
    actionFacade === "typed" &&
    typeof (entry.metadata as Record<string, unknown> | undefined)?.command_id !== "string";
  const persistedActionBlockReason = blockReasonFromEntry(entry, "action_block_reason");
  // F-21：终止判据缺省（undefined）回退通用判据；显式 null=终止放行（?? 会把
  // null 吞成回退，必须辨 undefined）。
  const persistedTerminateBlockReason =
    (entry.metadata as Record<string, unknown> | undefined)?.terminate_block_reason !== undefined
      ? blockReasonFromEntry(entry, "terminate_block_reason")
      : persistedActionBlockReason;
  // F-21：终止与确认共用「活投影匹配 + stage 前缀离场兜底」骨架，仅判据不同
  // （terminate_block_reason 允许 plan 会话 human_confirm 非终审门放行终止）。
  const actionBlockReason = useWorkspaceStore((state) =>
    gateCardBlockReason(state, entry, persistedActionBlockReason, (projection) =>
      projection.action_block_reason ?? null,
    ),
  );
  const terminateBlockReason = useWorkspaceStore((state) =>
    gateCardBlockReason(state, entry, persistedTerminateBlockReason, (projection) =>
      projection.terminate_block_reason !== undefined
        ? projection.terminate_block_reason
        : (projection.action_block_reason ?? null),
    ),
  );
  // C2（REQ-HGC-02 场景 1/F-54）：批次确认门专属标题/why——与候选修订门在
  // 标题/原因行层级可区分；确认动作走 confirmBatch（HTTP），无反馈编辑器。
  const isBatchConfirmGate = gateKindFromEntry(entry) === "batch_confirm";
  // C2K3 P3（REQ-HGC-01）：gate-local 已接受反馈轮次——CAS/serde/wire/store
  // 已端到端接线，此处补渲染消费（预算行「已反馈 N 轮 · 剩余 M」）。仅
  // plan 候选修订门当前卡消费；旧会话缺席（undefined/null）= 预算历史
  // 不可用，不猜（不渲染该段）。
  const acceptedFeedbackTurns = useWorkspaceStore((state) =>
    !isBatchConfirmGate && !isResolved && state.workspaceType === "work_item_plan"
      ? (state.humanGateSnapshot?.accepted_feedback_turns ?? null)
      : null,
  );
  // F-50 裁决 1（单标题制）：默认「需要人工确认」，仅 triage intent 门保留
  // 「需要判断 reviewer 意图」——标题回答「现在要做什么」，原因与建议交给
  // gate-why 单一原因行，不再三层同义。
  const title = isBatchConfirmGate
    ? gateBatchConfirmTitle()
    : requiresTriage
      ? "需要判断 reviewer 意图"
      : "需要人工确认";
  // F-50 裁决 2：gate-why = 单一原因行（含下一步建议）。triage 门给 intent
  // 原因行；其余按 findings 分级；无 findings 不猜（fail-closed）。
  const whyCopy = isBatchConfirmGate
    ? gateBatchConfirmWhyCopy()
    : requiresTriage
      ? GATE_TRIAGE_WHY_COPY
      : findings.length === 0
        ? null
        : requiredFindings.length > 0
          ? gateWhyRequiredCopy(requiredFindings.length)
          : gateWhyAdvisoryCopy(findings.length);
  // C2（REQ-HGC-02 场景 2）：距通过清单——优先用 durable 快照 findings（带
  // class，可判预检缺口），缺席退卡面 findings；仅 plan 候选修订门渲染。
  const snapshotFindings = useWorkspaceStore(
    (state) => state.humanGateSnapshot?.findings ?? null,
  );
  const workspaceTypeForDistance = useWorkspaceStore((state) => state.workspaceType);
  const distanceItems =
    !isBatchConfirmGate &&
    !isResolved &&
    workspaceTypeForDistance === "work_item_plan"
      ? gateDistanceToPass({
          findings: (snapshotFindings ?? findings).map((finding) =>
            "class" in finding && typeof finding.class === "string"
              ? { severity: finding.severity, class: finding.class }
              : { severity: finding.severity },
          ),
          trigger: gateTrigger,
          approveAvailable: actionBlockReason === null,
        })
      : null;
  // C2（REQ-HGC-02 场景 3）：跨轮 delta——current 取 durable 快照 findings
  // （C1 fingerprint+identity_unstable），previous 取 store 保留的上一轮快照
  // （缺席=刷新/历史不全 → 显式 unknown，不猜）；仅 plan 候选修订门渲染。
  const previousGateFindings = useWorkspaceStore(
    (state) => state.previousGateFindings,
  );
  const findingsDelta =
    !isBatchConfirmGate &&
    !isResolved &&
    workspaceTypeForDistance === "work_item_plan" &&
    snapshotFindings !== null
      ? gateFindingsCrossRoundDelta(snapshotFindings, previousGateFindings)
      : null;
  const archiveNote = archiveNoteFromEntry(entry);
  // F-49 B6：adoptable = advisory findings（must_fix 处理路径不同，不进默认采纳）。
  // 采纳按钮的目标是下方反馈输入框——编辑器不可用即无处可填，不露按钮（fail-closed）。
  const adoptableFindings = findings.filter((finding) => !isBlockingFinding(finding));
  const feedbackEditorVisible =
    !isResolved &&
    terminateBlockReason === null &&
    actions !== undefined &&
    actionFacade === "typed" &&
    actionBlockReason === null;
  const showAdoptFindingsButton = adoptableFindings.length > 0 && feedbackEditorVisible;
  // F-49 B4：门内修订进程（提交后「正在按反馈修订」→ 完成后「正在复评」）。只取
  // 「本卡就是当前 typed turn」的门卡（turn_id 匹配活 turn）——门内轮次切换后旧卡
  // 由 A1 收口为留档，不会误报进程。failed 交给失败行，不另报进程。
  const entryTurnId =
    (entry.metadata as Record<string, unknown> | undefined)?.turn_id;
  const liveTurnStatus = useWorkspaceStore((state) =>
    typeof entryTurnId === "string" && state.humanGateTurn?.turn_id === entryTurnId
      ? state.humanGateTurn.status
      : null,
  );
  const revisionProgressCopy =
    liveTurnStatus === "open" || liveTurnStatus === "busy"
      ? GATE_REVISION_RUNNING_NOTE
      : liveTurnStatus === "awaiting_confirm"
        ? GATE_REVISION_REVIEWING_NOTE
        : null;
  const handleFeedbackSubmit = (value: string) => {
    if (!actions || !actions.feedback(value)) {
      return;
    }
    // F-49 A4：提交成功即清空输入（此前文本留在框内，用户以为未提交），并把该轮
    // 反馈记到卡上——旧轮留档（B5）用它作摘要。
    recordGateFeedbackSubmission(entry.id, value);
    setFeedback("");
    setFeedbackSubmitted(true);
  };

  const handleAdoptFindings = () => {
    // F-49 B6：把 advisory findings 按模板填入下方反馈输入框——只填入不提交
    // （用户可继续编辑）；已有输入时以空格追加，防覆盖用户已写内容（反馈框是
    // 单行 input，value 净化会剥换行，不用换行连接）。F49-B6-fix1：同一
    // findings 的采纳文本是确定性的，草稿已含该段即跳过（防重复点击重复拼接）。
    const adopted = gateAdoptFindingsFeedback(adoptableFindings);
    setFeedback((current) =>
      current.includes(adopted)
        ? current
        : current.trim()
          ? `${current} ${adopted}`
          : adopted,
    );
  };

  return (
    <ChatEntryContainer
      role="system"
      title={title}
      titleSuffix={!isResolved ? <span className={GATE_CHIP_CLASS}>需人工</span> : undefined}
      // F-50 视觉 v2（f50-ui-visual-spec-v2 §1/§2）：门卡改中性白底+4px 琥珀左线
      // +chip——琥珀不再做整卡底色（琥珀压琥珀/零间距连片根因）；类常量与抽屉
      // 门禁条目共用（gate-visual-tokens.ts）。旧 gate-open token 面板退役。
      panelClassName={GATE_CARD_CLASS}
      titleClassName={GATE_TITLE_CLASS}
      testId="gate-prompt-entry"
    >
      {/* F-50 §4.1-3（第一批布局减负）：门卡固定「原因→正文→产物→证据→
          metadata→进度→局部错误→动作」顺序，确认对象（产物）不再被状态行压下去。 */}
      <div className="space-y-3">
        {whyCopy && !isResolved ? (
          <div
            data-testid="gate-why"
            className={`mt-1 text-sm font-medium ${GATE_SECONDARY_TEXT_CLASS}`}
          >
            {whyCopy}
          </div>
        ) : null}
        {/* F-50 裁决 1：entry.content/summary 与标题同义（人工介入同义句）时
            不渲染——单标题制下不让正文重复标题；独立事实保留。 */}
        {isGateTitleSynonymousCopy(entry.content) ? null : (
          <div className="text-sm text-slate-900">{entry.content}</div>
        )}
        {summary && !isGateTitleSynonymousCopy(summary) && summary !== entry.content ? (
          <div className={`text-xs ${GATE_META_TEXT_CLASS}`}>{summary}</div>
        ) : null}
        {!isResolved && pendingArtifactVersion && onOpenArtifact ? (
          <div
            data-testid="gate-artifact-context"
            className={`flex flex-wrap items-center gap-2 text-xs ${GATE_META_TEXT_CLASS}`}
          >
            <span>
              待确认产物：
              {GATE_ARTIFACT_LABELS[workspaceType ?? ""]
                ? `${GATE_ARTIFACT_LABELS[workspaceType ?? ""]} `
                : ""}
              v{pendingArtifactVersion.version}
            </span>
            <button
              type="button"
              data-testid="gate-artifact-open"
              onClick={onOpenArtifact}
              className={BTN_SECONDARY_CLASS}
            >
              <FileText className="h-4 w-4" aria-hidden="true" /> 查看产物
            </button>
          </div>
        ) : null}
        {findings.length > 0 && !isResolved ? (
          <details
            data-testid="gate-findings"
            className={`group ${GATE_NESTED_BLOCK_CLASS}`}
          >
            <summary
              className={`${DISCLOSURE_SUMMARY_CLASS} flex items-center gap-1 text-xs font-semibold text-slate-900`}
            >
              <ChevronRight className={DISCLOSURE_CHEVRON_CLASS} aria-hidden="true" />
              {gateFindingsToggleLabel(findings.length)}
            </summary>
            <div className="mt-2 space-y-2">
              {showAdoptFindingsButton ? (
                <div className="flex justify-end">
                  <button
                    type="button"
                    data-testid="gate-adopt-findings"
                    onClick={handleAdoptFindings}
                    className={BTN_SECONDARY_CLASS}
                  >
                    {GATE_ADOPT_FINDINGS_BUTTON_LABEL}
                  </button>
                </div>
              ) : null}
              <ReviewFindingGroups findings={findings} />
            </div>
          </details>
        ) : null}
        {/* C2（REQ-HGC-02 场景 3）：跨轮 delta 计数行——新增/已解决/复现，
            历史不全或身份 unstable 时如实显示 unknown（不猜）。 */}
        {findingsDelta ? (
          <p
            data-testid="gate-findings-delta"
            className={`text-xs font-medium ${GATE_META_TEXT_CLASS}`}
          >
            {gateFindingsDeltaCopy(findingsDelta)}
          </p>
        ) : null}
        {/* F-50 §4.1-4：trigger 与预算合并为一条弱化元数据行（不再两个胶囊
            抢主视觉）——「引擎判定需人工」不作为独立大胶囊重复人工介入语义。
            C2K3 P3：已接受反馈轮次并入本行（「已反馈 N 轮 · 剩余 M」），
            零轮次/旧会话缺席维持既有形态。 */}
        {gateTrigger || remainingBudget !== null || (acceptedFeedbackTurns ?? 0) > 0 ? (
          <p data-testid="gate-meta" className={`text-xs ${GATE_META_TEXT_CLASS}`}>
            {[
              gateTrigger ? `触发：${GATE_TRIGGER_LABELS[gateTrigger]}` : null,
              acceptedFeedbackTurns !== null && acceptedFeedbackTurns > 0
                ? `已反馈 ${acceptedFeedbackTurns} 轮`
                : null,
              remainingBudget !== null ? `剩余修复轮次 ${remainingBudget}` : null,
            ]
              .filter((part): part is string => Boolean(part))
              .join(" · ")}
          </p>
        ) : null}
        {/* C2（REQ-HGC-02 场景 2/F-52 §三）：「距通过」清单——从 durable
            findings/trigger/阻断判据派生的收敛路径；ok=已满足、pending=待处理、
            unknown=事实未同步（不猜）。 */}
        {distanceItems ? (
          <ul
            data-testid="gate-distance-to-pass"
            className={`space-y-1 text-xs ${GATE_META_TEXT_CLASS}`}
          >
            {distanceItems.map((item) => (
              <li key={item.key} data-testid={`gate-distance-${item.key}`}>
                {item.status === "ok" ? "✓" : item.status === "pending" ? "•" : "?"}{" "}
                {item.label}
              </li>
            ))}
          </ul>
        ) : null}
        {revisionProgressCopy && !isResolved ? (
          <div
            data-testid="gate-revision-status"
            className="text-xs font-medium text-slate-900"
          >
            {revisionProgressCopy}
          </div>
        ) : null}
        {/* F-50 §2.3-7/§2.7：门卡只保留与该门动作直接相关的局部错误（租约/
            连接级 hard error 由页级错误面与收件箱承载），置于动作区之前。 */}
        {failureMessage ? (
          <div
            data-testid="gate-failure"
            className="aria-mono text-xs text-[var(--aria-danger)]"
          >
            {failureMessage}
          </div>
        ) : null}
        {inlineError ? (
          <div
            data-testid="gate-inline-protocol-error"
            className="aria-mono text-xs text-[var(--aria-danger)]"
          >
            {inlineError.code} · {inlineError.message}
          </div>
        ) : null}
        {isContextBlockerGate && !isResolved ? (
          <div className={`text-xs ${GATE_META_TEXT_CLASS}`}>
            请在下方输入补充上下文后发送（对应 provide_context），或选择终止
          </div>
        ) : null}
        {isResolved ? (
          <div className="space-y-2">
            {archiveNote ? (
              <div data-testid="gate-archive-note" className={`text-xs ${GATE_META_TEXT_CLASS}`}>
                {archiveNote}
              </div>
            ) : null}
            <ResolutionBadge resolution={entry.resolution} />
          </div>
        ) : terminateBlockReason === null && actions ? (
          // F-21：终止放行即渲染动作位——phase_mismatch 的 plan 门（context
          // blocker/author 失败/缺相位）此前整面消失，终止零通路；现在露出
          // 终止（二次确认惯例），confirm/反馈编辑器维持相位纪律不渲染。
          // F-50 §4.1-5：动作区以分隔线成组，与说明区视觉分层。
          <div className={`space-y-2 ${GATE_DIVIDER_CLASS}`}>
            {/* C2（REQ-HGC-02/plan T2 ④，F-54 §4.2）：phase_mismatch 的通用相位
                提示行删除（0009 现场误导——编译失败残留被读成同步问题）；
                其余阻断理由（已离开门/已关闭）仍如实呈现。 */}
            {actionBlockReason !== null && actionBlockReason !== "phase_mismatch" ? (
              <p className={`text-xs ${GATE_META_TEXT_CLASS}`}>
                {gateActionBlockCopy(actionBlockReason)}，可终止后重新发起
              </p>
            ) : null}
            {actionFacade === "typed" && actionBlockReason === null ? (
              <>
                {/* F-50 裁决 3：反馈与确认并行——帮助文案明示反馈可选；修订
                    进行中隐藏静态指导（§2.3-5）。context blocker 门有自己的
                    专属指导，不叠两句。 */}
                {!revisionProgressCopy && !isContextBlockerGate ? (
                  <p
                    data-testid="gate-feedback-hint"
                    className={`text-xs ${GATE_META_TEXT_CLASS}`}
                  >
                    {GATE_FEEDBACK_OPTIONAL_HINT}
                  </p>
                ) : null}
                <GateFeedbackEditor
                  multiline={false}
                  value={feedback}
                  onChange={setFeedback}
                  onSubmit={handleFeedbackSubmit}
                />
              </>
            ) : null}
            {feedbackSubmitted && !revisionProgressCopy ? (
              <p
                data-testid="gate-feedback-submitted"
                className="text-xs font-medium text-emerald-700"
              >
                {GATE_FEEDBACK_SUBMITTED_NOTE}
              </p>
            ) : null}
            {typedGateAwaitingCommand && actionBlockReason === null ? (
              <p className={`text-xs ${GATE_META_TEXT_CLASS}`}>
                未同步门命令，将以新命令提交
              </p>
            ) : null}
            <div className="flex flex-wrap justify-end gap-2">
              {isContextBlockerGate || actionBlockReason !== null ? null : (
                // C2（REQ-HGC-02）：批次门确认=HTTP confirmBatch（发布语义）；
                // 候选修订门维持 WS confirm（确认当前版本）。
                <button
                  type="button"
                  onClick={() => {
                    if (isBatchConfirmGate) {
                      void actions.confirmBatch();
                    } else {
                      actions.confirm();
                    }
                  }}
                  className={BTN_PRIMARY_CLASS}
                >
                  <Check className="h-4 w-4" aria-hidden="true" />
                  {isBatchConfirmGate ? gateBatchConfirmTitle() : confirmLabel}
                </button>
              )}
              <ConfirmTwiceButton
                variant="ghost"
                label={GATE_TERMINATE_BUTTON_LABEL}
                confirmLabel={GATE_TERMINATE_CONFIRM_LABEL}
                onConfirm={actions.terminate}
              />
            </div>
          </div>
        ) : actionBlockReason && actionBlockReason !== "phase_mismatch" ? (
          <p className={`text-xs ${GATE_META_TEXT_CLASS}`}>
            {gateActionBlockCopy(actionBlockReason)}
          </p>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}

function ResolutionBadge({ resolution }: { resolution?: string }) {
  if (resolution === "confirm") {
    return (
      <span className="inline-flex items-center rounded-md bg-emerald-50 px-2 py-1 text-xs font-semibold text-emerald-700 ring-1 ring-emerald-200">
        已确认
      </span>
    );
  }
  if (resolution === "request-change") {
    return (
      <span className="inline-flex items-center rounded-md bg-amber-50 px-2 py-1 text-xs font-semibold text-amber-700 ring-1 ring-amber-200">
        已要求修改
      </span>
    );
  }
  if (resolution === "superseded") {
    return (
      <span
        data-testid="gate-archive-badge"
        className="inline-flex items-center rounded-md bg-slate-100 px-2 py-1 text-xs font-semibold text-slate-600 ring-1 ring-slate-200"
      >
        {GATE_ARCHIVE_BADGE_LABEL}
      </span>
    );
  }
  if (resolution === "terminate") {
    return (
      <span className="inline-flex items-center rounded-md bg-red-50 px-2 py-1 text-xs font-semibold text-red-700 ring-1 ring-red-200">
        已终止
      </span>
    );
  }
  return null;
}

function summaryFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.summary === "string" ? metadata.summary : null;
}

function verdictFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.verdict === "string" ? metadata.verdict : null;
}

function reviewGateFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.review_gate === "string" ? metadata.review_gate : null;
}

function gateKindFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.gate_kind === "string" ? metadata.gate_kind : null;
}

function blockReasonFromEntry(
  entry: ChatEntry,
  key: "action_block_reason" | "terminate_block_reason",
): Exclude<GateActionBlockReason, null> | null {
  const value = (entry.metadata as Record<string, unknown> | undefined)?.[key];
  return value === "terminal_stage" || value === "phase_mismatch" || value === "closed"
    ? value
    : null;
}

// 通用 stage 前缀门离场兜底（k3 P2-1）：门卡不随 stage_change 重建（仅
// setStage），阶段离开后投影消失——按 gateIdentity 与当前 stage 不一致判
// terminal_stage，避免重渲染出可点但被静默拦截的假按钮（human_confirm 与
// F-20 story/design author_confirm 门同款纪律）。
//
// F-49 A2：卡身份与当前门投影不一致即为「该卡不是当前门」——门内轮次切换时门载体
// 从 durable snapshot 切到 typed turn，旧卡 id 随之变化并经 upsert 留档；任何未
// 留档的残留卡都不得回退 persistedReason（= 开门时刻的 action_block_reason，实测
// null）放行动作：页面只挂一个动作门面，点旧卡实际作用于当前门（诊断 §0-B）。
function gateCardBlockReason(
  state: WorkspaceWsState,
  entry: ChatEntry,
  persistedReason: Exclude<GateActionBlockReason, null> | null,
  fromProjection: (projection: GateProjection) => Exclude<GateActionBlockReason, null> | null,
): Exclude<GateActionBlockReason, null> | null {
  const gateIdentity = (entry.metadata as Record<string, unknown> | undefined)?.gate_identity;
  if (typeof gateIdentity !== "string") {
    return persistedReason;
  }
  const projection = selectGateProjection(state);
  if (projection?.key === gateIdentity) {
    return projection.turn && state.stage !== "human_confirm"
      ? "terminal_stage"
      : fromProjection(projection);
  }
  // stage 前缀门（legacy story/design author_confirm）：身份即阶段，阶段变了才是
  // 离场；其余（typed turn / durable snapshot 载体）身份不匹配即被取代 → 已关闭。
  if (gateIdentity.startsWith("stage:")) {
    return gateIdentity === `stage:${state.stage}` ? persistedReason : "terminal_stage";
  }
  return "closed";
}
function gateTriggerFromEntry(
  entry: ChatEntry,
): WorkItemPlanHumanGateSnapshot["trigger"] | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const value = metadata?.gate_trigger;
  return value === "native_human_required" ||
    value === "repeated_fingerprint" ||
    value === "verification_new_findings" ||
    value === "repair_budget_exhausted"
    ? value
    : null;
}

function remainingBudgetFromEntry(entry: ChatEntry): number | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.remaining_budget === "number" ? metadata.remaining_budget : null;
}

function failureMessageFromEntry(entry: ChatEntry): string | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const failureClass =
    typeof metadata?.failure_class === "string" ? metadata.failure_class : null;
  const message = typeof metadata?.failure_message === "string" ? metadata.failure_message : null;
  if (!failureClass && !message) {
    return null;
  }
  return [failureClass, message].filter((part): part is string => Boolean(part)).join(" · ");
}

function inlineErrorFromEntry(entry: ChatEntry): { code: string; message: string } | null {
  const value = (entry.metadata as Record<string, unknown> | undefined)?.inline_error;
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const { code, message } = value as Record<string, unknown>;
  return typeof code === "string" && typeof message === "string" ? { code, message } : null;
}

// F-49 B5：旧轮留档文案（由 store 的 upsertGatePromptEntry 写入 metadata）。留档卡
// 是只读的——不复用 action_block_reason 面，只呈现轮次与摘要。
function archiveNoteFromEntry(entry: ChatEntry): string | null {
  const value = (entry.metadata as Record<string, unknown> | undefined)?.gate_archive_note;
  return typeof value === "string" ? value : null;
}
