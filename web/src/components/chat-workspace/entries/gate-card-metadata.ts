// 门卡 entry metadata 的读取器（从 GatePromptEntry.tsx 纯移动拆出——该文件逼近
// large_file_guard 的 1200 行红线；此处只做「读 metadata → 事实」的解析，零行为
// 变化、无状态、无副作用）。
//
// 纪律：字段缺席一律显式 null/缺省值，绝不猜——门卡的判定（阻断、身份、预算、
// 失败、留档）全部以此处解析出的事实为唯一来源。
import type { WorkItemPlanHumanGateSnapshot } from "../../../api/types";
import type { ChatEntry } from "../../../state/chat-entries";
import type { GateActionBlockReason } from "../../../state/workspace-cockpit-projection";

export function summaryFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.summary === "string" ? metadata.summary : null;
}

export function verdictFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.verdict === "string" ? metadata.verdict : null;
}

export function reviewGateFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.review_gate === "string" ? metadata.review_gate : null;
}

export function gateKindFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.gate_kind === "string" ? metadata.gate_kind : null;
}

export function blockReasonFromEntry(
  entry: ChatEntry,
  key: "action_block_reason" | "terminate_block_reason",
): Exclude<GateActionBlockReason, null> | null {
  const value = (entry.metadata as Record<string, unknown> | undefined)?.[key];
  return value === "terminal_stage" || value === "phase_mismatch" || value === "closed"
    ? value
    : null;
}

export function gateTriggerFromEntry(
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

/**
 * 门卡的门身份（buildGatePromptEntry 写入 metadata.gate_identity）。
 *
 * REQ-HGC-01「卡片刷新」的判据输入：身份与当前门投影一致 = 本卡是当前门
 * （读同门当前事实）；不一致 = 旧卡（只读历史事实）。
 */
export function gateIdentityFromEntry(entry: ChatEntry): string | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const value = metadata?.gate_identity;
  return typeof value === "string" ? value : null;
}

export function remainingBudgetFromEntry(entry: ChatEntry): number | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  return typeof metadata?.remaining_budget === "number" ? metadata.remaining_budget : null;
}

export function failureMessageFromEntry(entry: ChatEntry): string | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const failureClass =
    typeof metadata?.failure_class === "string" ? metadata.failure_class : null;
  const failureMessage =
    typeof metadata?.failure_message === "string" ? metadata.failure_message : null;
  if (!failureClass && !failureMessage) {
    return null;
  }
  return [failureClass, failureMessage].filter((part): part is string => Boolean(part)).join(" · ");
}

export function inlineErrorFromEntry(entry: ChatEntry): { code: string; message: string } | null {
  const value = (entry.metadata as Record<string, unknown> | undefined)?.inline_error;
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const { code, message } = value as Record<string, unknown>;
  return typeof code === "string" && typeof message === "string" ? { code, message } : null;
}

// F-49 B5：旧轮留档文案（由 store 的 upsertGatePromptEntry 写入 metadata）。留档卡
// 是只读的——不复用 action_block_reason 面，只呈现轮次与摘要。
export function archiveNoteFromEntry(entry: ChatEntry): string | null {
  const value = (entry.metadata as Record<string, unknown> | undefined)?.gate_archive_note;
  return typeof value === "string" ? value : null;
}
