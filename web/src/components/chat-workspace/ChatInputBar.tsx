import {
  Check,
  ClipboardCopy,
  GitBranch,
  Layers,
  Play,
  RefreshCcw,
  Send,
  X,
} from "lucide-react";
import { useState, type FormEvent } from "react";
import type {
  AuthorDecisionChoice,
  WorkItemBatchDecision,
  WorkItemDraftDecision,
  WorkItemGenerationMode,
  WorkItemPlanArtifactPayload,
} from "../../api/types";
import { useWorkspaceStore } from "../../state/workspace-ws-store";
import type { ChatEntry, ChatEntryType } from "../../state/chat-entries";
import { DraftValidationFailureNotice } from "../workspace/DraftValidationFailureNotice";

interface ChatInputBarProps {
  stage: string;
  activeNodeType?: string | null;
  workItemPlanArtifact?: WorkItemPlanArtifactPayload | null;
  onSendContextNote: (content: string) => void;
  onStartGeneration: () => void;
  onSendHumanDecision: (payload: { description: string; source: "human" }) => void;
  onAuthorDecision?: (decision: AuthorDecisionChoice, feedback?: string) => void;
  onSelectWorkItemGenerationMode?: (mode: WorkItemGenerationMode) => void;
  onRequestOutlineRevision?: () => void;
  onWorkItemDraftDecision?: (outlineId: string, decision: WorkItemDraftDecision) => void;
  onWorkItemBatchDecision?: (
    decision: WorkItemBatchDecision,
    feedback?: string,
    firstAffectedOutlineId?: string,
  ) => void;
  onAbort: () => void;
  disabled?: boolean;
  hideStartGeneration?: boolean;
  /** author_confirm 阶段三动作的默认高亮：启用 review 时高亮「确认并送审」，否则「确认定稿」。 */
  reviewerEnabled?: boolean;
  /** adopt-review-findings T1：最新 review 报告文本（与对话流渲染同源）；非空时展示「采纳 Review 意见」。 */
  latestReviewReport?: string;
}

const BUSY_STAGES = new Set(["running", "cross_review", "revision"]);

export function ChatInputBar({
  stage,
  activeNodeType = null,
  workItemPlanArtifact = null,
  onSendContextNote,
  onStartGeneration,
  onSendHumanDecision,
  onAuthorDecision = () => undefined,
  onSelectWorkItemGenerationMode = () => undefined,
  onRequestOutlineRevision = () => undefined,
  onWorkItemDraftDecision = () => undefined,
  onWorkItemBatchDecision = () => undefined,
  onAbort,
  disabled = false,
  hideStartGeneration = false,
  reviewerEnabled = true,
  latestReviewReport,
}: ChatInputBarProps) {
  const [input, setInput] = useState("");
  const trimmedInput = input.trim();
  const isPrepareContext = stage === "prepare_context";
  const isAuthorConfirm = stage === "author_confirm";
  const isWorkItemOutlineConfirm = activeNodeType === "work_item_plan_outline_confirm";
  const isWorkItemGenerationMode = activeNodeType === "work_item_generation_mode";
  const isWorkItemDraftConfirm = activeNodeType === "work_item_draft_confirm";
  const isWorkItemBatchConfirm = activeNodeType === "work_item_batch_confirm";
  const isHumanConfirm = stage === "human_confirm";
  const isBusy = BUSY_STAGES.has(stage);
  // spec-design-dialog-revision T8：author_confirm 反馈输入开放（原为禁用）；
  // 发送仍走「发送反馈」按钮而非表单提交。
  const inputDisabled = disabled || isBusy || stage === "completed";
  const canSend = !inputDisabled && (isPrepareContext || isHumanConfirm) && trimmedInput.length > 0;
  const showSend = isPrepareContext || isHumanConfirm;
  const draftPayload =
    workItemPlanArtifact?.type === "draft_candidate" ? workItemPlanArtifact.payload : null;
  const batchPayload =
    workItemPlanArtifact?.type === "batch_state" ? workItemPlanArtifact.payload : null;
  const firstBatchFailureOutlineId = batchPayload?.failure_summary[0]?.outline_id;

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!canSend) {
      return;
    }

    if (isHumanConfirm) {
      useWorkspaceStore.getState().resolveGateEntry("request-change");
      appendOptimisticEntry("human_decision", trimmedInput);
      onSendHumanDecision({ description: trimmedInput, source: "human" });
    } else {
      appendOptimisticEntry("context_note", trimmedInput);
      onSendContextNote(trimmedInput);
    }
    setInput("");
  }

  function handleStartGeneration() {
    if (disabled) {
      return;
    }
    appendOptimisticEntry("start_generation", "开始生成");
    onStartGeneration();
  }

  function handleSendAuthorFeedback() {
    if (disabled || trimmedInput.length === 0) {
      return;
    }
    onAuthorDecision("revise", trimmedInput);
    setInput("");
  }

  // adopt-review-findings T1：覆盖式预填引导语 + 最新 review 报告；仅预填，不自动发送，
  // 重复点击天然不拼接（spec Scenario 3）。
  function handleAdoptReviewReport() {
    if (disabled || !latestReviewReport) {
      return;
    }
    setInput(`按以下 review 意见修订：\n\n${latestReviewReport}`);
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
          disabled={inputDisabled}
          rows={3}
          placeholder={placeholderForStage(stage, activeNodeType)}
          className="min-h-20 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        />
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
              data-testid={isHumanConfirm ? "send-human-decision" : "send-context-note"}
              type="submit"
              disabled={!canSend}
              className="btn-secondary h-9 disabled:opacity-50"
            >
              <Send className="h-4 w-4" />
              {isHumanConfirm ? "发送修改意见" : "发送"}
            </button>
          ) : null}
          {isWorkItemOutlineConfirm ? (
            <>
              <button
                type="button"
                onClick={() => onRequestOutlineRevision()}
                disabled={disabled}
                className="btn-secondary h-9 disabled:opacity-50"
              >
                <RefreshCcw className="h-4 w-4" />
                重写 Outline
              </button>
              <button
                type="button"
                onClick={() => onAuthorDecision("accept")}
                disabled={disabled}
                className="btn-primary h-9 disabled:opacity-50"
              >
                <Check className="h-4 w-4" />
                接受 Outline
              </button>
            </>
          ) : isWorkItemGenerationMode ? (
            <>
              <button
                type="button"
                onClick={() => onSelectWorkItemGenerationMode("serial")}
                disabled={disabled}
                className="btn-secondary h-9 disabled:opacity-50"
              >
                <GitBranch className="h-4 w-4" />
                逐个生成
              </button>
              <button
                type="button"
                onClick={() => onSelectWorkItemGenerationMode("batch")}
                disabled={disabled}
                className="btn-primary h-9 disabled:opacity-50"
              >
                <Layers className="h-4 w-4" />
                自动生成
              </button>
              <button
                type="button"
                onClick={onRequestOutlineRevision}
                disabled={disabled}
                className="btn-secondary h-9 disabled:opacity-50"
              >
                <RefreshCcw className="h-4 w-4" />
                返回 Outline 返修
              </button>
            </>
          ) : isWorkItemDraftConfirm ? (
            <>
              {!draftPayload?.can_accept ? (
                <DraftValidationFailureNotice findings={draftPayload?.validator_findings} />
              ) : null}
              {draftPayload?.can_accept ? (
                <button
                  type="button"
                  onClick={() =>
                    onWorkItemDraftDecision(draftPayload.draft_record.outline_id, "accept")
                  }
                  disabled={disabled}
                  className="btn-primary h-9 disabled:opacity-50"
                >
                  <Check className="h-4 w-4" />
                  接受
                </button>
              ) : null}
              {draftPayload ? (
                <>
                  <button
                    type="button"
                    onClick={() =>
                      onWorkItemDraftDecision(draftPayload.draft_record.outline_id, "rewrite")
                    }
                    disabled={disabled}
                    className="btn-secondary h-9 disabled:opacity-50"
                  >
                    <RefreshCcw className="h-4 w-4" />
                    {draftPayload.can_accept ? "重写" : "根据校验错误重写"}
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      onWorkItemDraftDecision(draftPayload.draft_record.outline_id, "pause")
                    }
                    disabled={disabled}
                    className="btn-secondary h-9 disabled:opacity-50"
                  >
                    <X className="h-4 w-4" />
                    暂停
                  </button>
                </>
              ) : null}
            </>
          ) : isWorkItemBatchConfirm ? (
            <>
              <button
                type="button"
                onClick={() => onWorkItemBatchDecision("accept_all")}
                disabled={disabled}
                className="btn-primary h-9 disabled:opacity-50"
              >
                <Check className="h-4 w-4" />
                接受全部
              </button>
              <button
                type="button"
                onClick={() => onWorkItemBatchDecision("rewrite_batch")}
                disabled={disabled}
                className="btn-secondary h-9 disabled:opacity-50"
              >
                <RefreshCcw className="h-4 w-4" />
                整组重写
              </button>
              <button
                type="button"
                onClick={() => onWorkItemBatchDecision("pause")}
                disabled={disabled}
                className="btn-secondary h-9 disabled:opacity-50"
              >
                <X className="h-4 w-4" />
                暂停
              </button>
              {firstBatchFailureOutlineId ? (
                <button
                  type="button"
                  onClick={() =>
                    onWorkItemBatchDecision(
                      "downgrade_to_serial",
                      undefined,
                      firstBatchFailureOutlineId,
                    )
                  }
                  disabled={disabled}
                  className="btn-secondary h-9 disabled:opacity-50"
                >
                  <GitBranch className="h-4 w-4" />
                  降级串行
                </button>
              ) : null}
            </>
          ) : isAuthorConfirm ? (
            <>
              {latestReviewReport ? (
                <button
                  type="button"
                  onClick={handleAdoptReviewReport}
                  disabled={disabled}
                  className="btn-secondary h-9 disabled:opacity-50"
                >
                  <ClipboardCopy className="h-4 w-4" />
                  采纳 Review 意见
                </button>
              ) : null}
              <button
                type="button"
                onClick={handleSendAuthorFeedback}
                disabled={disabled || trimmedInput.length === 0}
                className="btn-secondary h-9 disabled:opacity-50"
              >
                <Send className="h-4 w-4" />
                发送反馈
              </button>
              <button
                type="button"
                onClick={() => onAuthorDecision("accept_with_review")}
                disabled={disabled}
                className={
                  reviewerEnabled
                    ? "btn-primary h-9 disabled:opacity-50"
                    : "btn-secondary h-9 disabled:opacity-50"
                }
              >
                <GitBranch className="h-4 w-4" />
                确认并送审
              </button>
              <button
                type="button"
                onClick={() => onAuthorDecision("accept_finalize")}
                disabled={disabled}
                className={
                  reviewerEnabled
                    ? "btn-secondary h-9 disabled:opacity-50"
                    : "btn-primary h-9 disabled:opacity-50"
                }
              >
                <Check className="h-4 w-4" />
                确认定稿
              </button>
            </>
          ) : null}
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
}

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
    return "输入修改意见...";
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
