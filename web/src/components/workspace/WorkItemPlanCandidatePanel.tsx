import { useMemo, useState } from "react";
import type { WorkItemPlanCandidateDto, WorkItemCandidateDto } from "../../api/types";

export interface WorkItemPlanCandidatePanelProps {
  candidate: WorkItemPlanCandidateDto;
  stage: string;
  onRevert: (workItemId: string, feedback: string, clear: boolean) => void;
  onRequestRevision: (feedback?: string) => void;
  onAccept: () => void;
  className?: string;
}

export function WorkItemPlanCandidatePanel({
  candidate,
  stage,
  onRevert,
  onRequestRevision,
  onAccept,
  className = "",
}: WorkItemPlanCandidatePanelProps) {
  const [revertingId, setRevertingId] = useState<string | null>(null);
  const [revertFeedback, setRevertFeedback] = useState("");

  const revertedCount = useMemo(
    () => candidate.work_items.filter((item) => item.reverted).length,
    [candidate.work_items],
  );

  const isAuthorConfirm = stage === "author_confirm";

  function startRevert(item: WorkItemCandidateDto) {
    setRevertingId(item.candidate_id);
    setRevertFeedback(item.revert_feedback ?? "");
  }

  function submitRevert(itemId: string) {
    onRevert(itemId, revertFeedback, false);
    setRevertingId(null);
    setRevertFeedback("");
  }

  function clearRevert(itemId: string) {
    onRevert(itemId, "", true);
  }

  function cancelRevert() {
    setRevertingId(null);
    setRevertFeedback("");
  }

  return (
    <div
      data-testid="work-item-plan-candidate-panel"
      className={`flex min-h-0 flex-col gap-4 overflow-auto p-4 ${className}`}
    >
      <h2 className="text-sm font-semibold text-[var(--aria-ink)]">Work Item Plan 候选</h2>

      {candidate.plan.dependency_graph.length > 0 ? (
        <section data-testid="candidate-dependency-dag">
          <h3 className="mb-2 text-xs font-semibold text-[var(--aria-ink-muted)]">依赖关系</h3>
          <ul className="space-y-1 text-sm text-[var(--aria-ink)]">
            {candidate.plan.dependency_graph.map((edge, index) => (
              <li key={`${edge.from_work_item_id}-${edge.to_work_item_id}-${index}`}>
                {edge.from_work_item_id} → {edge.to_work_item_id}
                <span className="ml-2 text-xs text-[var(--aria-ink-muted)]">
                  ({edge.dependency_type})
                </span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <section data-testid="candidate-work-items">
        <h3 className="mb-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
          Work Items ({candidate.work_items.length})
        </h3>
        <div className="space-y-3">
          {candidate.work_items.map((item) => (
            <WorkItemCard
              key={item.candidate_id}
              item={item}
              isAuthorConfirm={isAuthorConfirm}
              isReverting={revertingId === item.candidate_id}
              revertFeedback={revertFeedback}
              onRevertFeedbackChange={setRevertFeedback}
              onStartRevert={() => startRevert(item)}
              onSubmitRevert={() => submitRevert(item.candidate_id)}
              onClearRevert={() => clearRevert(item.candidate_id)}
              onCancelRevert={cancelRevert}
            />
          ))}
        </div>
      </section>

      {candidate.repository_profile ? (
        <section data-testid="candidate-repository-profile">
          <h3 className="mb-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
            Repository Profile
          </h3>
          <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3 text-sm">
            <div className="flex items-center gap-2">
              <span className="text-[var(--aria-ink-muted)]">置信度</span>
              <span
                className="rounded px-1.5 py-0.5 text-xs font-semibold"
                style={confidenceStyle(candidate.repository_profile.confidence)}
              >
                {candidate.repository_profile.confidence}
              </span>
            </div>
            <div className="mt-2">
              <span className="text-[var(--aria-ink-muted)]">检测到的分层：</span>
              <span className="ml-1 text-[var(--aria-ink)]">
                {candidate.repository_profile.detected_layers.join(", ") || "--"}
              </span>
            </div>
          </div>
        </section>
      ) : null}

      {candidate.validator_findings.length > 0 ? (
        <section data-testid="candidate-validator-findings">
          <h3 className="mb-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
            Validator Findings
          </h3>
          <ul className="space-y-2">
            {candidate.validator_findings.map((finding) => (
              <li
                key={finding.finding_id}
                className="rounded-md border border-[var(--aria-line)] p-3 text-sm"
                style={findingStyle(finding.level)}
              >
                <div className="flex items-center gap-2">
                  <span className="text-xs font-semibold uppercase">{finding.level}</span>
                  {finding.code ? (
                    <span className="font-mono text-xs text-[var(--aria-ink-muted)]">
                      {finding.code}
                    </span>
                  ) : null}
                </div>
                <p className="mt-1 text-[var(--aria-ink)]">{finding.message}</p>
                {finding.affected_scopes.length > 0 ? (
                  <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
                    影响范围：{finding.affected_scopes.join(", ")}
                  </p>
                ) : null}
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {isAuthorConfirm ? (
        <div className="mt-auto flex items-center gap-3 border-t border-[var(--aria-line)] pt-4">
          <button
            type="button"
            data-testid="request-revision-button"
            disabled={revertedCount === 0}
            onClick={() => onRequestRevision()}
            className="inline-flex h-9 items-center rounded-md bg-amber-100 px-3 text-sm font-semibold text-amber-800 hover:bg-amber-200 disabled:cursor-not-allowed disabled:opacity-50"
          >
            重新生成被标记的 {revertedCount} 项
          </button>
          <button
            type="button"
            data-testid="accept-plan-button"
            onClick={onAccept}
            className="inline-flex h-9 items-center rounded-md bg-emerald-100 px-3 text-sm font-semibold text-emerald-800 hover:bg-emerald-200"
          >
            确认计划
          </button>
        </div>
      ) : null}
    </div>
  );
}

interface WorkItemCardProps {
  item: WorkItemCandidateDto;
  isAuthorConfirm: boolean;
  isReverting: boolean;
  revertFeedback: string;
  onRevertFeedbackChange: (value: string) => void;
  onStartRevert: () => void;
  onSubmitRevert: () => void;
  onClearRevert: () => void;
  onCancelRevert: () => void;
}

function WorkItemCard({
  item,
  isAuthorConfirm,
  isReverting,
  revertFeedback,
  onRevertFeedbackChange,
  onStartRevert,
  onSubmitRevert,
  onClearRevert,
  onCancelRevert,
}: WorkItemCardProps) {
  return (
    <div
      data-testid={`work-item-candidate-${item.candidate_id}`}
      className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3"
      style={{ opacity: item.reverted ? 0.6 : 1 }}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="text-sm font-semibold text-[var(--aria-ink)]">{item.title}</span>
            <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5 text-xs text-[var(--aria-ink-muted)]">
              {item.kind}
            </span>
          </div>
          <div className="mt-2 space-y-1 text-xs text-[var(--aria-ink-muted)]">
            <div>独占写范围：{item.exclusive_write_scopes.join(", ") || "--"}</div>
            <div>依赖：{item.depends_on.join(", ") || "无"}</div>
            <div>验证计划：{item.verification_plan_ref ?? "--"}</div>
          </div>
          {item.reverted ? (
            <div className="mt-2 text-xs text-amber-700">
              已标记撤销
              {item.revert_feedback ? `：${item.revert_feedback}` : ""}
            </div>
          ) : null}
        </div>
        {isAuthorConfirm ? (
          <div className="shrink-0">
            {item.reverted ? (
              <button
                type="button"
                data-testid={`clear-revert-${item.candidate_id}`}
                onClick={onClearRevert}
                className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] px-2 text-xs hover:bg-[var(--aria-panel-muted)]"
              >
                取消标记
              </button>
            ) : isReverting ? null : (
              <button
                type="button"
                data-testid={`start-revert-${item.candidate_id}`}
                onClick={onStartRevert}
                className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] px-2 text-xs hover:bg-[var(--aria-panel-muted)]"
              >
                Revert
              </button>
            )}
          </div>
        ) : null}
      </div>

      {isReverting ? (
        <div className="mt-3 space-y-2">
          <input
            type="text"
            data-testid={`revert-feedback-input-${item.candidate_id}`}
            value={revertFeedback}
            onChange={(event) => onRevertFeedbackChange(event.target.value)}
            placeholder="请输入 revert 反馈"
            className="w-full rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm outline-none focus:border-[var(--aria-primary)]"
          />
          <div className="flex items-center gap-2">
            <button
              type="button"
              data-testid={`submit-revert-${item.candidate_id}`}
              onClick={onSubmitRevert}
              className="inline-flex h-7 items-center rounded-md bg-amber-100 px-2 text-xs font-semibold text-amber-800 hover:bg-amber-200"
            >
              提交 Revert
            </button>
            <button
              type="button"
              data-testid={`cancel-revert-${item.candidate_id}`}
              onClick={onCancelRevert}
              className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] px-2 text-xs hover:bg-[var(--aria-panel-muted)]"
            >
              取消
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function confidenceStyle(confidence: string): React.CSSProperties {
  switch (confidence) {
    case "high":
      return { backgroundColor: "#d1fae5", color: "#065f46" };
    case "medium":
      return { backgroundColor: "#fef3c7", color: "#92400e" };
    case "low":
      return { backgroundColor: "#fee2e2", color: "#991b1b" };
    default:
      return { backgroundColor: "#f3f4f6", color: "#374151" };
  }
}

function findingStyle(level: string): React.CSSProperties {
  switch (level) {
    case "error":
      return { backgroundColor: "#fef2f2", borderColor: "#fecaca" };
    case "warning":
      return { backgroundColor: "#fffbeb", borderColor: "#fde68a" };
    default:
      return { backgroundColor: "#f9fafb", borderColor: "#e5e7eb" };
  }
}
