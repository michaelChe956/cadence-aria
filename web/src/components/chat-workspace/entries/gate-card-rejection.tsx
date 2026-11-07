// 从 GatePromptEntry.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 根因 E（REQ-WSC-15 前端面）——validation_reject 门 turn 的可操作化呈现：被拒
// 反馈、可读原因、下一步动作与逐字诊断。此处只做「读 metadata → 事实/渲染」，
// 无状态、无副作用。
import type { ChatEntry } from "../../../state/chat-entries";
import { humanizeGateProse } from "./gate-next-step";

/** 根因 E：validation_reject 可操作化呈现所需的最小事实（全部可缺席）。 */
export type GateRejectionInfo = {
  rejectedFeedback: string;
  rejectionReason: string;
  rejectionNextActions: string[];
  rejectionDiagnostics: string[];
};

export function gateRejectionInfoFromEntry(entry: ChatEntry): GateRejectionInfo | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const rejectedFeedback =
    typeof metadata?.rejectedFeedback === "string" ? metadata.rejectedFeedback : null;
  // owner 2026-10-30：被拒原因是 REQ-WSC-14「可读原因」——已知机器码映射，
  // 未解释术语逐字保留（jargon: "keep"，不套黑话兜底：整段/片段改写都会
  // 丢真实原因）；下一步动作仍按默认片段摘除；rejectedFeedback 是 owner
  // 自己的话、diagnostics 是逐字诊断，均保持原文。
  const rejectionReason =
    typeof metadata?.rejectionReason === "string"
      ? humanizeGateProse(metadata.rejectionReason, { jargon: "keep" })
      : null;
  const rejectionNextActions = Array.isArray(metadata?.rejectionNextActions)
    ? metadata.rejectionNextActions
        .filter((item): item is string => typeof item === "string")
        .map((item) => humanizeGateProse(item))
    : [];
  const rejectionDiagnostics = Array.isArray(metadata?.rejectionDiagnostics)
    ? metadata.rejectionDiagnostics.filter((item): item is string => typeof item === "string")
    : [];
  if (
    !rejectedFeedback &&
    !rejectionReason &&
    rejectionNextActions.length === 0 &&
    rejectionDiagnostics.length === 0
  ) {
    return null;
  }
  return {
    rejectedFeedback: rejectedFeedback ?? "",
    rejectionReason: rejectionReason ?? "",
    rejectionNextActions,
    rejectionDiagnostics,
  };
}

/**
 * 根因 E（REQ-WSC-15 前端面）：validation_reject 门 turn 的可操作化呈现——
 * 此前失败门只有 `failure_class · failure_message` 一行，owner 不知道被拒的是
 * 哪条反馈、为什么、下一步做什么（现场卡死主因）。被拒反馈与原因可读呈现，
 * 诊断逐字（不改写、不截断）。
 */
export function GateRejectionBlock({ info }: { info: GateRejectionInfo }) {
  return (
    <section
      data-testid="gate-rejection-block"
      className="space-y-2 rounded-md border border-red-200 bg-red-50 px-3 py-2"
    >
      <div className="text-xs font-semibold text-red-800">上一条反馈被拒收</div>
      {info.rejectedFeedback ? (
        <div data-testid="gate-rejected-feedback" className="text-sm text-slate-900">
          被拒反馈：{info.rejectedFeedback}
        </div>
      ) : null}
      {info.rejectionReason ? (
        <div data-testid="gate-rejection-reason" className="text-sm text-slate-900">
          原因：{info.rejectionReason}
        </div>
      ) : null}
      {info.rejectionNextActions.length > 0 ? (
        <ul
          data-testid="gate-rejection-next-actions"
          className="list-disc space-y-1 pl-5 text-sm text-slate-900"
        >
          {info.rejectionNextActions.map((action, index) => (
            <li key={`${action}-${index}`}>{action}</li>
          ))}
        </ul>
      ) : null}
      {info.rejectionDiagnostics.length > 0 ? (
        <pre
          data-testid="gate-rejection-diagnostics"
          className="aria-mono overflow-x-auto whitespace-pre-wrap text-xs text-[var(--aria-ink)]"
        >
          {info.rejectionDiagnostics.join("\n")}
        </pre>
      ) : null}
    </section>
  );
}
