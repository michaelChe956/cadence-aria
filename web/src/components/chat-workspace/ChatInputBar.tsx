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
  useState,
  type FormEvent,
} from "react";
import type {
  WorkItemPlanArtifactPayload,
} from "../../api/types";
import { useWorkspaceStore } from "../../state/workspace-ws-store";
import type { ChatEntry, ChatEntryType } from "../../state/chat-entries";
import { ConfirmTwiceButton } from "./cockpit/ConfirmTwiceButton";
import { DraftValidationFailureNotice } from "../workspace/DraftValidationFailureNotice";

interface ChatInputBarProps {
  stage: string;
  activeNodeType?: string | null;
  workItemPlanArtifact?: WorkItemPlanArtifactPayload | null;
  onSendContextNote: (content: string) => void;
  onStartGeneration: () => void;
  // L2 退役（T5/REQ-RET-02）：staged/author 决策回调（outline 确认/生成模式/
  // outline 返修/draft/batch/author）随 wire 消息族删除——对应按钮分支退役。
  onAbort: () => void;
  disabled?: boolean;
  hideStartGeneration?: boolean;
  /** spec-workbench-canvas-experience T4：输入框聚焦回调（并存面板据此收起）。 */
  onInputFocus?: () => void;
  /** F-28（v33 复验 3）：STALE_DRIVER_LEASE 就地错误面——直出在生成动作区
   * （不受收件箱 actionable 条件限制），重接管复用 F-11 二次确认回调链。 */
  staleLeaseNotice?: { message: string; onRetakeLease: () => void } | null;
}

/**
 * spec-workbench-canvas-experience T4：暴露预填能力给宿主页面——
 * 「采纳 Review 意见」按钮已迁移至 ArtifactReviewPanel，预填仍复用本组件输入框状态。
 */
export interface ChatInputBarHandle {
  prefill: (text: string) => void;
}

const BUSY_STAGES = new Set(["running", "cross_review", "revision"]);

export const ChatInputBar = forwardRef<ChatInputBarHandle, ChatInputBarProps>(
  function ChatInputBar({
  stage,
  activeNodeType = null,
  workItemPlanArtifact = null,
  onSendContextNote,
  onStartGeneration,
  onAbort,
  disabled = false,
  hideStartGeneration = false,
  onInputFocus,
  staleLeaseNotice = null,
}, ref) {
  const [input, setInput] = useState("");
  const trimmedInput = input.trim();
  const isPrepareContext = stage === "prepare_context";
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
  const inputDisabled =
    disabled || isHumanConfirm || isBusy || stage === "completed";
  const canSend = !inputDisabled && isPrepareContext && trimmedInput.length > 0;
  const showSend = isPrepareContext;
  const draftPayload =
    workItemPlanArtifact?.type === "draft_candidate" ? workItemPlanArtifact.payload : null;
  const batchPayload =
    workItemPlanArtifact?.type === "batch_state" ? workItemPlanArtifact.payload : null;
  const firstBatchFailureOutlineId = batchPayload?.failure_summary[0]?.outline_id;

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
    appendOptimisticEntry("context_note", trimmedInput);
    onSendContextNote(trimmedInput);
    setInput("");
  }

  function handleStartGeneration() {
    if (disabled) {
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
          data-testid="context-note-input"
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onFocus={onInputFocus}
          disabled={inputDisabled}
          rows={3}
          placeholder={placeholderForStage(stage, activeNodeType)}
          className="min-h-20 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        />
        {/* F-28：丢租约后就地错误面紧贴动作按钮行——收件箱条目远离视线导致
            零反馈；重接管复用 F-11 的 ConfirmTwiceButton 二次确认纪律。 */}
        {staleLeaseNotice ? (
          <div
            data-testid="stale-lease-notice"
            role="alert"
            className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-red-200 bg-red-50 px-3 py-2"
          >
            <span className="text-xs font-semibold text-red-700">
              连接租约已过期——本连接的写操作已被拒绝（STALE_DRIVER_LEASE）：{staleLeaseNotice.message}
            </span>
            <ConfirmTwiceButton
              label="重新接管"
              confirmLabel="确认重新接管"
              onConfirm={staleLeaseNotice.onRetakeLease}
            />
          </div>
        ) : null}
        <div className="flex flex-wrap justify-end gap-2">
          {isBusy ? (
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
              data-testid="send-context-note"
              type="submit"
              disabled={!canSend}
              className="btn-secondary h-9 disabled:opacity-50"
            >
              <Send className="h-4 w-4" />
              发送
            </button>
          ) : null}
          {/* 退役留档（T5/REQ-RET-02）：staged/author 决策按钮分支随消息族删除。 */}
          {isPrepareContext && !hideStartGeneration ? (
            <button
              data-testid="start-generation"
              type="button"
              onClick={handleStartGeneration}
              disabled={disabled}
              className="btn-primary h-9 disabled:opacity-50"
            >
              <Play className="h-4 w-4" />
              开始生成
            </button>
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
