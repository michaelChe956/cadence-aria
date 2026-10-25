import type { ChatEntry } from "../state/chat-entries";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";

// 从 CodingWorkspacePage.tsx 拆出（large_file_guard 行数上限，纯移动零行为
// 变化）：choice 卡元数据读写、执行计划引用归一、阶段文案与整组删除命令号。
export function choiceEntryFromState(requestId: string) {
  return (
    useCodingWorkspaceStore
      .getState()
      .chatEntries.find(
        (candidate) =>
          candidate.type === "choice_request" && candidate.metadata?.request_id === requestId,
      ) ?? null
  );
}

export function choiceMetadataString(entry: ChatEntry | null, key: string) {
  const metadata = entry?.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.[key] === "string" ? (metadata[key] as string) : null;
}

export function markChoiceEntry(
  requestId: string,
  merge: (metadata: Record<string, unknown>) => Record<string, unknown>,
  resolved = false,
) {
  const store = useCodingWorkspaceStore.getState();
  const entry = choiceEntryFromState(requestId);
  if (!entry) return;
  store.appendChatEntry({
    ...entry,
    resolved,
    metadata: merge({ ...(entry.metadata as Record<string, unknown>) }),
  });
}

export function normalizedRefs(refs: string[] | undefined): string[] {
  return [...new Set((refs ?? []).map((ref) => ref.trim()).filter(Boolean))].sort();
}

export function displayCodingStage(stage: string) {
  const labels: Record<string, string> = {
    prepare_context: "准备上下文",
    worktree_prepare: "准备 Worktree",
    coding: "Coder",
    code_review: "Code Reviewer",
    review_request: "准备 PR",
    internal_pr_review: "GroupFinalReview",
    final_confirm: "人工组最终确认",
  };
  return labels[stage] ?? stage;
}

// H5：整组删除一次用户动作生成一次（同动作失败重试复用，服务端按 command_id
// 幂等回执；与 workbench 的 plan_start_ui_ 同一约定）。
export function newPlanGroupDeleteCommandId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `plan_delete_ui_${crypto.randomUUID()}`;
  }
  return `plan_delete_ui_${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}
