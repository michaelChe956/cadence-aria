import { CircleCheckBig, FileText, FlaskConical, GitCommitHorizontal } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

/**
 * 平台确定性交付摘要(Q4③):unit 完成后由平台事实生成的「交付事实」尾帧。
 * 字段全部来自平台记录(completion_commit、start..completion 变更文件、
 * 验证证据、handoff id),不依赖 agent 汇报 prose。
 */
export function DeliveryFactsEntry({ entry }: { entry: ChatEntry }) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const completionCommit =
    typeof metadata?.completion_commit === "string" ? metadata.completion_commit : null;
  const startCommit = typeof metadata?.start_commit === "string" ? metadata.start_commit : null;
  const changedFiles = Array.isArray(metadata?.changed_files)
    ? metadata.changed_files.filter((file): file is string => typeof file === "string")
    : [];
  const verification = (metadata?.verification ?? null) as Record<string, unknown> | null;
  const reviewReportId =
    verification && typeof verification.review_report_id === "string"
      ? verification.review_report_id
      : null;
  const verdict =
    verification && typeof verification.verdict === "string" ? verification.verdict : null;
  const evidenceRefs =
    verification && Array.isArray(verification.tested_evidence_refs)
      ? verification.tested_evidence_refs.filter((ref): ref is string => typeof ref === "string")
      : [];
  const handoffRevisionId =
    typeof metadata?.handoff_revision_id === "string" ? metadata.handoff_revision_id : null;
  const logicalWorkItemId =
    typeof metadata?.logical_work_item_id === "string" ? metadata.logical_work_item_id : null;

  return (
    <ChatEntryContainer role="system" title="交付事实(平台生成)">
      <div className="space-y-2 text-sm text-[var(--aria-ink)]">
        {logicalWorkItemId ? (
          <div className="flex items-center gap-2">
            <CircleCheckBig className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
            <span className="font-medium">{entry.content || `${logicalWorkItemId} 交付完成`}</span>
          </div>
        ) : null}
        <div className="flex items-start gap-2">
          <GitCommitHorizontal className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
          <div className="min-w-0">
            <div className="font-mono text-xs break-all">{completionCommit ?? "无提交记录"}</div>
            {startCommit && startCommit !== completionCommit ? (
              <div className="mt-0.5 font-mono text-xs text-[var(--aria-ink-muted)]">
                {startCommit}..{completionCommit}
              </div>
            ) : null}
          </div>
        </div>
        <div className="flex items-start gap-2">
          <FileText className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
          <div className="min-w-0">
            <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">
              变更文件({changedFiles.length})
            </div>
            {changedFiles.length > 0 ? (
              <ul className="mt-1 space-y-0.5">
                {changedFiles.map((file) => (
                  <li key={file} className="font-mono text-xs break-all">
                    {file}
                  </li>
                ))}
              </ul>
            ) : (
              <div className="mt-0.5 text-xs text-[var(--aria-ink-muted)]">无</div>
            )}
          </div>
        </div>
        <div className="flex items-start gap-2">
          <FlaskConical className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
          <div className="min-w-0">
            <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">
              验证证据({evidenceRefs.length})
            </div>
            <div className="mt-0.5 text-xs">
              {verdict ? `code review ${verdict}` : "无 code review 记录"}
              {reviewReportId ? ` · ${reviewReportId}` : ""}
            </div>
            {evidenceRefs.length > 0 ? (
              <ul className="mt-1 space-y-0.5">
                {evidenceRefs.map((ref) => (
                  <li key={ref} className="font-mono text-xs break-all">
                    {ref}
                  </li>
                ))}
              </ul>
            ) : null}
          </div>
        </div>
        {handoffRevisionId ? (
          <div className="text-xs text-[var(--aria-ink-muted)]">
            handoff: <span className="font-mono break-all">{handoffRevisionId}</span>
          </div>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}
