import { Check, FileText } from "lucide-react";
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
  GATE_ADVISORY_CONFIRM_HINT,
  GATE_ARCHIVE_BADGE_LABEL,
  GATE_FEEDBACK_SUBMITTED_NOTE,
  GATE_REVISION_REVIEWING_NOTE,
  GATE_REVISION_RUNNING_NOTE,
  gateAdoptFindingsFeedback,
  gateFindingsToggleLabel,
  gateWhyAdvisoryCopy,
  gateWhyRequiredCopy,
} from "../../../state/gate-prompt-copy";
import type { WorkspaceWsState } from "../../../state/workspace-ws-store-types";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import type { WorkItemPlanHumanGateSnapshot } from "../../../api/types";
import { WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND } from "../../../state/workspace-chat-rebuild";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { ConfirmTwiceButton } from "../cockpit/ConfirmTwiceButton";
import { GateFeedbackEditor } from "../cockpit/GateFeedbackEditor";
import { ChatEntryContainer } from "../ChatEntryContainer";
import { isRequiredFindingSeverity, ReviewFindingGroups, reviewFindingsFromEntry } from "../finding-list";

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
  const requiredFindings = findings.filter((finding) =>
    isRequiredFindingSeverity(finding.severity),
  );
  const needsHuman = verdict === "needs_human";
  const requiresTriage = reviewGate === "user_triage_required";
  const allowsCurrentVersion = reviewGate === "user_confirm_allowed";
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
  const confirmLabel =
    requiresTriage
      ? "确认当前版本"
      : allowsCurrentVersion
      ? "确认使用当前版本"
      : needsHuman
        ? "提交人工确认"
        : "确认产物";
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
  const title = requiresTriage
    ? "需要判断 reviewer 意图"
    : allowsCurrentVersion
      ? "可确认当前版本"
      : needsHuman
        ? "需要人工确认"
        : "人工确认";
  // F-49 B1/B2：门卡首行必须回答「为什么需要你」与「建议确认还是反馈」——此前只有
  // 4 字 trigger chip，确认者拿不到决策依据（文案见 gate-prompt-copy.ts）。无 findings
  // 即不猜（fail-closed）；建议行只在确认按钮确实露出（无阻断、非 context blocker 门）
  // 时才给「可直接确认」的建议。
  const whyCopy =
    findings.length === 0
      ? null
      : requiredFindings.length > 0
        ? gateWhyRequiredCopy(requiredFindings.length)
        : gateWhyAdvisoryCopy(findings.length);
  const archiveNote = archiveNoteFromEntry(entry);
  const confirmOffered = !isResolved && actionBlockReason === null && !isContextBlockerGate;
  const advisoryOnly = findings.length > 0 && requiredFindings.length === 0;
  // F-49 B6：adoptable = advisory findings（must_fix 处理路径不同，不进默认采纳）。
  // 采纳按钮的目标是下方反馈输入框——编辑器不可用即无处可填，不露按钮（fail-closed）。
  const adoptableFindings = findings.filter(
    (finding) => !isRequiredFindingSeverity(finding.severity),
  );
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
    // 单行 input，value 净化会剥换行，不用换行连接）。
    const adopted = gateAdoptFindingsFeedback(adoptableFindings);
    setFeedback((current) => (current.trim() ? `${current} ${adopted}` : adopted));
  };

  return (
    <ChatEntryContainer
      role="system"
      title={title}
      className="border-slate-200 bg-slate-50"
      testId="gate-prompt-entry"
    >
      <div className="space-y-3">
        {whyCopy && !isResolved ? (
          <div
            data-testid="gate-why"
            className="text-sm font-medium text-[var(--aria-ink)]"
          >
            {whyCopy}
          </div>
        ) : null}
        {advisoryOnly && confirmOffered ? (
          <div data-testid="gate-advice" className="text-xs text-emerald-700">
            {GATE_ADVISORY_CONFIRM_HINT}
          </div>
        ) : null}
        {findings.length > 0 && !isResolved ? (
          <details
            data-testid="gate-findings"
            className="rounded-md border border-[var(--aria-line)] bg-white px-3 py-2"
          >
            <summary className="cursor-pointer text-xs font-semibold text-[var(--aria-ink)]">
              {gateFindingsToggleLabel(findings.length)}
            </summary>
            <div className="mt-2 space-y-2">
              {showAdoptFindingsButton ? (
                <div className="flex justify-end">
                  <button
                    type="button"
                    data-testid="gate-adopt-findings"
                    onClick={handleAdoptFindings}
                    className="inline-flex min-h-9 items-center gap-1 rounded-md border border-[var(--aria-line-strong)] bg-white px-2 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                  >
                    {GATE_ADOPT_FINDINGS_BUTTON_LABEL}
                  </button>
                </div>
              ) : null}
              <ReviewFindingGroups findings={findings} />
            </div>
          </details>
        ) : null}
        <div className="text-sm text-[var(--aria-ink)]">{entry.content}</div>
        {summary ? <div className="text-xs text-[var(--aria-ink-muted)]">{summary}</div> : null}
        {!isResolved && pendingArtifactVersion && onOpenArtifact ? (
          <div
            data-testid="gate-artifact-context"
            className="flex flex-wrap items-center gap-2 text-xs text-[var(--aria-ink-muted)]"
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
              className="inline-flex min-h-9 items-center gap-1 rounded-md border border-[var(--aria-line-strong)] bg-white px-2 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            >
              <FileText className="h-3.5 w-3.5" aria-hidden="true" /> 查看产物
            </button>
          </div>
        ) : null}
        {gateTrigger || remainingBudget !== null ? (
          <div className="flex flex-wrap items-center gap-2 text-xs">
            {gateTrigger ? (
              <span
                data-testid="gate-trigger-label"
                className="aria-chip border-[var(--aria-gate-open-border)] bg-[var(--aria-gate-open-bg)] text-[var(--aria-gate-open-fg)]"
              >
                {GATE_TRIGGER_LABELS[gateTrigger]}
              </span>
            ) : null}
            {remainingBudget !== null ? (
              <span
                data-testid="gate-budget"
                className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
              >
                剩余修复轮次 {remainingBudget}
              </span>
            ) : null}
          </div>
        ) : null}
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
        {revisionProgressCopy && !isResolved ? (
          <div
            data-testid="gate-revision-status"
            className="text-xs font-medium text-[var(--aria-ink)]"
          >
            {revisionProgressCopy}
          </div>
        ) : null}
        {requiresTriage && findings.length === 0 ? (
          <div className="text-xs text-[var(--aria-ink-muted)]">
            请在下方输入人工修改说明后发送返修。
          </div>
        ) : null}
        {isContextBlockerGate && !isResolved ? (
          <div className="text-xs text-[var(--aria-ink-muted)]">
            请在下方输入补充上下文后发送（对应 provide_context），或选择终止
          </div>
        ) : null}
        {isResolved ? (
          <div className="space-y-2">
            {archiveNote ? (
              <div data-testid="gate-archive-note" className="text-xs text-[var(--aria-ink-muted)]">
                {archiveNote}
              </div>
            ) : null}
            <ResolutionBadge resolution={entry.resolution} />
          </div>
        ) : terminateBlockReason === null && actions ? (
          // F-21：终止放行即渲染动作位——phase_mismatch 的 plan 门（context
          // blocker/author 失败/缺相位）此前整面消失，终止零通路；现在露出
          // 终止（二次确认惯例），confirm/反馈编辑器维持相位纪律不渲染。
          <div className="space-y-2">
            {actionBlockReason !== null ? (
              <p className="text-xs text-[var(--aria-ink-muted)]">
                {gateActionBlockCopy(actionBlockReason)}，可终止后重新发起
              </p>
            ) : null}
            {actionFacade === "typed" && actionBlockReason === null ? (
              <GateFeedbackEditor
                multiline={false}
                value={feedback}
                onChange={setFeedback}
                onSubmit={handleFeedbackSubmit}
              />
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
              <p className="text-xs text-[var(--aria-ink-muted)]">
                未同步门命令，将以新命令提交
              </p>
            ) : null}
            <div className="flex flex-wrap justify-end gap-2">
              {isContextBlockerGate || actionBlockReason !== null ? null : (
                <button
                  type="button"
                  onClick={() => actions.confirm()}
                  className="inline-flex min-h-11 items-center gap-1 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                >
                  <Check className="h-3.5 w-3.5" aria-hidden="true" />
                  {confirmLabel}
                </button>
              )}
              <ConfirmTwiceButton
                label="终止"
                confirmLabel="确认终止"
                onConfirm={actions.terminate}
              />
            </div>
          </div>
        ) : actionBlockReason ? (
          <p className="text-xs text-[var(--aria-ink-muted)]">
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
