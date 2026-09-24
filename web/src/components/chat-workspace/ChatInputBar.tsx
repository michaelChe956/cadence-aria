import {
  Check,
  GitBranch,
  Layers,
  Play,
  RefreshCcw,
  Send,
  X,
} from "lucide-react";
import {
  forwardRef,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import type {
  WorkItemPlanArtifactPayload,
} from "../../api/types";
import { useWorkspaceStore } from "../../state/workspace-ws-store";
import { getProviderOption } from "../../state/provider-options";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import type { ChatEntry, ChatEntryType } from "../../state/chat-entries";
import { ConfirmTwiceButton } from "./cockpit/ConfirmTwiceButton";
import { PROTOCOL_ERROR_DETAILS_LABEL, protocolErrorCopy } from "../../state/protocol-error-copy";
import { DraftValidationFailureNotice } from "../workspace/DraftValidationFailureNotice";
import { LeaseDiagnosticsSummary } from "./LeaseDiagnosticsSummary";
import type { LeaseDiagnosticsEvent } from "../../state/lease-diagnostics";

interface ChatInputBarProps {
  stage: string;
  activeNodeType?: string | null;
  workItemPlanArtifact?: WorkItemPlanArtifactPayload | null;
  onSendContextNote: (content: string) => void;
  /** v38 复验 #2/#3（恢复 C3 前原意）：story/design AuthorConfirm 门的反馈
   * 修订发送通道——宿主（story/design 会话）接线时 author_confirm 输入+「发送
   * 反馈」可用，提交即 request_revision；返回 false 表示未发出（如断线），
   * 输入保留不落乐观条目。WorkItemPlan 门/未接线时该阶段保持只读呈现。 */
  onSendRevisionFeedback?: (feedback: string) => boolean;
  onStartGeneration: () => void;
  // L2 退役（T5/REQ-RET-02）：staged/author 决策回调（outline 确认/生成模式/
  // outline 返修/draft/batch/author）随 wire 消息族删除——对应按钮分支退役。
  onAbort: () => void;
  disabled?: boolean;
  hideStartGeneration?: boolean;
  /** F-30（v34 复验 session_0008）：终态会话（confirmed/terminated）上禁用
   * 「开始生成」并就地展示如实提示（重跑走修订流程或新建会话）。 */
  startGenerationDisabled?: boolean;
  startGenerationDisabledHint?: string | null;
  /** spec-workbench-canvas-experience T4：输入框聚焦回调（并存面板据此收起）。 */
  onInputFocus?: () => void;
  /** F-28 二轮（v34 复验「失败态点开始生成零反馈」）：hard_error 全族就地
   * 错误面——直出在生成动作区（不受收件箱 actionable 条件限制）。lease
   * 拒收两码（STALE_DRIVER_LEASE/OBSERVER_WRITE_REJECTED）附重接管二次确认；
   * 其余码只报错误码与建议刷新。 */
  hardErrorNotice?: HardErrorNotice | null;
  /** C3/REQ-HTR-02：人工门开态（门投影存在）不渲染 run 级中止钮——stage
   * 漂移停在 busy 形态（F-53 现场）时中止钮被误当会话级「终止」；门级
   * 动作（反馈/确认/终止此门）由门卡承载，此处只做呈现分层不发新帧。 */
  gateOpen?: boolean;
}

export interface HardErrorNotice {
  code: string;
  message: string;
  /** lease 拒收两码提供：重发 driver hello 夺回租约（复用 F-11 回调链）。 */
  onRetakeLease: (() => void) | null;
  /**
   * F-50 裁决 7：抽屉打开承载完整错误动作时，页级缩为引用面——显示指向
   * 待处理抽屉的提示并不再渲染重接管按钮；null 表示页级自持完整动作面。
   */
  referenceNote?: string | null;
  /**
   * REQ-DLS-03：STALE 错误面引用的最近租约转移事件（诊断端点拉取）。
   * null/缺省（未激活或端点失败静默降级）不渲染摘要。
   */
  leaseEvents?: readonly LeaseDiagnosticsEvent[] | null;
}

/**
 * spec-workbench-canvas-experience T4：暴露预填能力给宿主页面——
 * 「采纳 Review 意见」按钮已迁移至 ArtifactReviewPanel，预填仍复用本组件输入框状态。
 */
export interface ChatInputBarHandle {
  prefill: (text: string) => void;
}
const BUSY_STAGES = new Set(["running", "cross_review", "revision"]);

/** 3.6 滚动体系收敛：textarea 高度下限/上限，与 min-h-20 / max-h-60 对齐。 */
const TEXTAREA_MIN_HEIGHT = 80;
const TEXTAREA_MAX_HEIGHT = 240;

export const ChatInputBar = forwardRef<ChatInputBarHandle, ChatInputBarProps>(
  function ChatInputBar({
  stage,
  activeNodeType = null,
  workItemPlanArtifact = null,
  onSendContextNote,
  onSendRevisionFeedback,
  onStartGeneration,
  onAbort,
  disabled = false,
  hideStartGeneration = false,
  startGenerationDisabled = false,
  startGenerationDisabledHint = null,
  onInputFocus,
  hardErrorNotice = null,
  gateOpen = false,
}, ref) {
  const [input, setInput] = useState("");
  const trimmedInput = input.trim();
  const isPrepareContext = stage === "prepare_context";
  // REQ-PPS-03：开始生成前必须可见「将要使用的实际 author provider」及其可用性；不可用即
  // fail-closed 禁用按钮并给出来因。author 兜底与 providerConfigFor 同源（同为
  // claude_code）；可用性快照由 ProviderAvailabilityGuard 在这棵子树渲染前加载完成，
  // 未加载时如实标「可用性未知」而不假装 provider 不可用。
  const providers = useWorkspaceStore((state) => state.providers);
  const providerSnapshot = useProviderAvailabilityStore((state) => state.snapshot);
  const authorProvider = providers?.author ?? "claude_code";
  const authorOption = providerSnapshot
    ? getProviderOption(providerSnapshot, authorProvider)
    : null;
  const providerBlockReason =
    authorOption && !authorOption.available
      ? `${authorOption.label} 当前不可用：${authorOption.reason ?? "原因未知"}`
      : null;
  const providerStatusSuffix = !authorOption
    ? "（可用性未知）"
    : authorOption.available
      ? "（可用）"
      : "（不可用）";
  // 生成动作区只在 PrepareContext 有意义：Running 等阶段（生成中/锁定）不受 provider 门影响。
  const showStartGeneration = isPrepareContext && !hideStartGeneration;
  const startGenerationBlocked =
    startGenerationDisabled || providerBlockReason !== null;
  // 阻断提示：provider 不可用原因优先（按钮可见时才谈得上），其次宿主传入的终态提示；
  // 与「开始生成」显隐无关——可恢复中断把按钮藏起时，F-30 的如实提示仍就地可见。
  const generateBlockedHint = !isPrepareContext
    ? null
    : (showStartGeneration ? providerBlockReason : null) ??
      (startGenerationDisabled ? startGenerationDisabledHint : null);
  const isAuthorConfirm = stage === "author_confirm";
  const isWorkItemOutlineConfirm = activeNodeType === "work_item_plan_outline_confirm";
  const isWorkItemGenerationMode = activeNodeType === "work_item_generation_mode";
  const isWorkItemDraftConfirm = activeNodeType === "work_item_draft_confirm";
  const isWorkItemBatchConfirm = activeNodeType === "work_item_batch_confirm";
  // L1 重承载（REQ-RET-02）：human_confirm 决策输入发送面随旧协议退役删除——
  // 该阶段无发送通道，输入只读呈现（决策走 typed 门动作面）。
  const isHumanConfirm = stage === "human_confirm";
  const isBusy = BUSY_STAGES.has(stage);
  // spec-design-dialog-revision T8：author_confirm 反馈输入开放（原为禁用）；
  // 发送仍走「发送反馈」按钮而非表单提交。
  // v38 复验 #2/#3：story/design 宿主接线修订回调后，author_confirm 门上
  // 「发送反馈」恢复可用（提交即 request_revision）；未接线（WorkItemPlan 门/
  // 旧页面）时维持只读呈现，不发不误发。
  const inputDisabled =
    disabled || isHumanConfirm || isBusy || stage === "completed";
  const canSendRevision =
    isAuthorConfirm && onSendRevisionFeedback !== undefined && !inputDisabled;
  const canSend =
    !inputDisabled && trimmedInput.length > 0 && (isPrepareContext || canSendRevision);
  const showSend = isPrepareContext || canSendRevision;
  const draftPayload =
    workItemPlanArtifact?.type === "draft_candidate" ? workItemPlanArtifact.payload : null;
  const batchPayload =
    workItemPlanArtifact?.type === "batch_state" ? workItemPlanArtifact.payload : null;
  const firstBatchFailureOutlineId = batchPayload?.failure_summary[0]?.outline_id;

  // 3.6 滚动体系收敛：textarea 不再以 rows=3 固定高度制造小内滚区——高度随
  // 内容自适应（80–240px），封顶后超出部分才在框内滚动；resize-y 保留。
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  useLayoutEffect(() => {
    const el = textareaRef.current;
    if (!el) {
      return;
    }
    el.style.height = "auto";
    // border-box 下 scrollHeight 不含上下边框（border=1px），补齐避免 2px 差再触发滚动条
    const measured = el.scrollHeight + 2;
    el.style.height = `${Math.min(Math.max(measured, TEXTAREA_MIN_HEIGHT), TEXTAREA_MAX_HEIGHT)}px`;
  }, [input]);

  useImperativeHandle(
    ref,
    () => ({
      // adopt-review-findings：覆盖式预填（重复调用天然不拼接）。
      prefill: (text: string) => setInput(text),
    }),
    [],
  );

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!canSend) {
      return;
    }
    if (isPrepareContext) {
      appendOptimisticEntry("context_note", trimmedInput);
      onSendContextNote(trimmedInput);
      setInput("");
      return;
    }
    // v38 #2/#3：门上反馈修订——发出才算消耗（断线 false 保留输入，反馈不丢）。
    if (onSendRevisionFeedback?.(trimmedInput) !== false) {
      appendOptimisticEntry("context_note", trimmedInput);
      setInput("");
    }
  }

  function handleStartGeneration() {
    if (disabled || startGenerationBlocked) {
      return;
    }
    appendOptimisticEntry("start_generation", "开始生成");
    onStartGeneration();
  }

  return (
    <form
      data-testid="chat-input-bar"
      onSubmit={handleSubmit}
      className="border-t border-[var(--aria-line)] bg-[var(--aria-panel)] p-3"
    >
      <div className="flex min-w-0 flex-col gap-2">
        <textarea
          ref={textareaRef}
          data-testid="context-note-input"
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onFocus={onInputFocus}
          disabled={inputDisabled}
          rows={3}
          placeholder={placeholderForStage(stage, activeNodeType)}
          className="min-h-20 max-h-60 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        />
        {/* F-28 二轮：hard_error 就地错误面紧贴动作按钮行——收件箱条目远离
            视线导致零反馈；lease 拒收两码的重接管复用 F-11 的
            ConfirmTwiceButton 二次确认纪律，其余码报错误码+建议刷新。 */}
        {hardErrorNotice ? (
          // F-50 裁决 6/7：中文主显 lead + mono 错误码副行 + 中文正文；英文原文
          // 进折叠详情（无译文时原文主显，不藏信息）。抽屉打开时（referenceNote）
          // 页级缩为引用面——只报状态与处理入口，不渲染重接管动作。
          (() => {
            const errorCopy = protocolErrorCopy(hardErrorNotice.code);
            const body = errorCopy.body ?? hardErrorNotice.message;
            const hasTranslation = errorCopy.body !== null;
            return (
              <div
                data-testid="hard-error-notice"
                role="alert"
                className="rounded-md border border-red-200 bg-red-50 px-3 py-2"
              >
                <div className="flex min-w-0 flex-1 flex-col gap-1">
                  <p className="flex flex-wrap items-baseline gap-x-2">
                    <span className="text-xs font-semibold text-red-700">{errorCopy.lead}</span>
                    <span
                      data-testid="hard-error-code"
                      className="aria-mono text-xs font-normal text-red-600"
                    >
                      {hardErrorNotice.code}
                    </span>
                  </p>
                  <p className="text-xs leading-4 text-red-700">
                    {body}
                    {hardErrorNotice.referenceNote != null
                      ? `——${hardErrorNotice.referenceNote}`
                      : hardErrorNotice.onRetakeLease === null
                        ? "——建议刷新页面或重新进入会话后重试"
                        : null}
                  </p>
                  {hardErrorNotice.leaseEvents ? (
                    <LeaseDiagnosticsSummary events={hardErrorNotice.leaseEvents} />
                  ) : null}
                  {hasTranslation ? (
                    <details data-testid="hard-error-details">
                      <summary className="cursor-pointer text-xs font-medium text-red-600">
                        {PROTOCOL_ERROR_DETAILS_LABEL}
                      </summary>
                      <p className="aria-mono mt-1 break-words text-xs leading-4 text-red-600">
                        {hardErrorNotice.message}
                      </p>
                    </details>
                  ) : null}
                </div>
                {hardErrorNotice.onRetakeLease !== null ? (
                  <ConfirmTwiceButton
                    label="重新接管"
                    confirmLabel="确认重新接管"
                    onConfirm={hardErrorNotice.onRetakeLease}
                  />
                ) : null}
              </div>
            );
          })()
        ) : null}
        <div className="flex flex-wrap items-center justify-end gap-2">
          {/* C3/REQ-HTR-02：busy 形态叠加「无开启中人工门」——门开态（含
              stage 漂移误显 busy）不渲染 run 级中止钮，避免挤占「终止」心智。 */}
          {isBusy && !gateOpen ? (
            <button
              type="button"
              onClick={onAbort}
              disabled={disabled}
              className="inline-flex h-9 items-center gap-2 rounded-md border border-red-200 bg-red-50 px-3 text-sm font-semibold text-red-700 hover:bg-red-100 disabled:opacity-50"
            >
              <X className="h-4 w-4" />
              中止
            </button>
          ) : null}
          {showSend ? (
            <button
              data-testid={canSendRevision ? "send-revision-feedback" : "send-context-note"}
              type="submit"
              disabled={!canSend}
              className="btn-secondary h-9 disabled:opacity-50"
            >
              <Send className="h-4 w-4" />
              {canSendRevision ? "发送反馈" : "发送"}
            </button>
          ) : null}
          {/* 退役留档（T5/REQ-RET-02）：staged/author 决策按钮分支随消息族删除。 */}
          {showStartGeneration ? (
            <span
              data-testid="start-generation-provider"
              className="inline-flex h-9 items-center text-xs font-semibold text-[var(--aria-ink-muted)]"
            >
              {`Provider：${authorOption?.label ?? authorProvider}${providerStatusSuffix}`}
            </span>
          ) : null}
          {showStartGeneration ? (
            <button
              data-testid="start-generation"
              type="button"
              onClick={handleStartGeneration}
              disabled={disabled || startGenerationBlocked}
              className="btn-primary h-9 disabled:opacity-50"
            >
              <Play className="h-4 w-4" />
              开始生成
            </button>
          ) : null}
          {generateBlockedHint ? (
            <p
              data-testid="start-generation-blocked-hint"
              className="w-full text-right text-xs text-[var(--aria-ink-muted)]"
            >
              {generateBlockedHint}
            </p>
          ) : null}
        </div>
      </div>
    </form>
  );
});

function placeholderForStage(stage: string, activeNodeType?: string | null) {
  if (activeNodeType === "work_item_plan_outline_confirm") {
    return "请确认 WorkItemPlan Outline";
  }
  if (activeNodeType === "work_item_generation_mode") {
    return "请选择 Work Item 生成模式";
  }
  if (activeNodeType === "work_item_draft_confirm") {
    return "请确认当前 Work Item Draft";
  }
  if (activeNodeType === "work_item_batch_confirm") {
    return "请确认整组 Work Item Draft";
  }
  if (stage === "human_confirm") {
    return "人工确认阶段（决策走门禁操作）";
  }
  if (stage === "author_confirm") {
    // spec-design-dialog-revision T8：推倒重来出口移除，反馈修订成为主路径。
    return "输入修改意见，或直接确认";
  }
  if (BUSY_STAGES.has(stage)) {
    return "Provider 运行中，暂不可输入";
  }
  if (stage === "completed") {
    return "流程已完成";
  }
  return "补充上下文";
}

function appendOptimisticEntry(type: ChatEntryType, content: string) {
  const entry: ChatEntry = {
    id: `${type}:optimistic:${Date.now()}`,
    type,
    role: type === "start_generation" ? "system" : "user",
    content,
    timestamp: new Date().toISOString(),
  };
  useWorkspaceStore.getState().appendChatEntry(entry);
}


