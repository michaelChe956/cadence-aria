import { Check } from "lucide-react";
import { useState } from "react";
import {
  confirmWorkItemExecutionPlan,
  requestWorkItemExecutionPlanChange,
} from "../api/client";
import type {
  AnalystDecisionRecord,
  CodeReviewReport,
  InternalPrReview,
  ReviewFinding,
  TestingStepResult,
  WorkItemExecutionPlan,
} from "../api/types";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { errorMessage } from "./CodingWorkspaceControls";

export function TestsPanel() {
  const report = useCodingWorkspaceStore((state) => state.testingReport);
  const stage = useCodingWorkspaceStore((state) => state.stage);
  const latestAnalystDecision = useCodingWorkspaceStore(
    (state) => state.latestAnalystDecision,
  );
  if (!report) {
    return <div className="text-[var(--aria-ink-muted)]">暂无测试报告</div>;
  }
  const steps = report.steps ?? [];
  const missingRequiredSteps = report.missing_required_steps ?? [];
  const skippedRequiredSteps = report.skipped_required_steps ?? [];
  const contextWarnings = report.context_warnings ?? [];
  const unplannedCommands = report.unplanned_commands ?? [];
  const unplannedEvidence = report.unplanned_evidence ?? [];
  const hasPlanDetails =
    Boolean(report.plan_summary) ||
    steps.length > 0 ||
    missingRequiredSteps.length > 0 ||
    skippedRequiredSteps.length > 0 ||
    contextWarnings.length > 0 ||
    Boolean(report.raw_provider_output_ref);

  return (
    <div className="space-y-3">
      <StatusBadge value={report.overall_status} />
      <AnalystDecisionStatus
        decision={latestAnalystDecision}
        waiting={stage === "rework" && !latestAnalystDecision}
      />
      {hasPlanDetails ? (
        <div data-testid="coding-test-plan-report" className="space-y-2">
          {report.plan_summary ? (
            <div>
              <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">
                Test Plan
              </div>
              <div className="mt-0.5 break-words text-sm font-semibold">
                {report.plan_summary}
              </div>
            </div>
          ) : null}
          {steps.length > 0 ? (
            <div className="space-y-2">
              {steps.map((step) => (
                <TestingStepResultRow key={step.step_id} step={step} />
              ))}
            </div>
          ) : null}
          <TestingList label="missing required" values={missingRequiredSteps} />
          <TestingList label="skipped required" values={skippedRequiredSteps} />
          <TestingList label="context warning" values={contextWarnings} />
          {report.raw_provider_output_ref ? (
            <EvidencePath label="raw output" value={report.raw_provider_output_ref} />
          ) : null}
          {unplannedEvidence.length > 0 ? (
            <div className="space-y-1">
              <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">
                unplanned evidence
              </div>
              {unplannedEvidence.map((evidence) => (
                <div
                  key={evidence.tool_use_id}
                  className="rounded-md border border-[var(--aria-line)] p-2 text-xs"
                >
                  <div className="font-mono">{evidence.tool_name}</div>
                  <div className="text-[var(--aria-ink-muted)]">{evidence.status}</div>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
      {[...report.commands, ...unplannedCommands].map((command, index) => (
        <div
          key={`${command.command.join(" ")}-${index}`}
          className="rounded-md border border-[var(--aria-line)] p-2"
        >
          <div className="break-words font-mono text-xs">{command.command.join(" ")}</div>
          <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
            {command.status} · exit {command.exit_code ?? "-"} · {command.duration_ms}ms
          </div>
        </div>
      ))}
    </div>
  );
}

function AnalystDecisionStatus({
  decision,
  waiting,
}: {
  decision: AnalystDecisionRecord | null;
  waiting: boolean;
}) {
  if (!decision) {
    return waiting ? (
      <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
        等待 Analyst 决策
      </div>
    ) : null;
  }

  return (
    <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs">
      <div className="font-semibold text-[var(--aria-ink)]">Analyst 已决策</div>
      <div className="mt-1 font-mono text-[var(--aria-ink-muted)]">
        {decision.verdict} {"->"} {decision.next_stage}
      </div>
      <div className="mt-1 break-words text-[var(--aria-ink)]">{decision.reason}</div>
      <TestingList label="analyst evidence" values={decision.evidence_refs} />
      {decision.raw_provider_output_refs.length > 0 ? (
        <TestingList label="analyst raw" values={decision.raw_provider_output_refs} />
      ) : null}
    </div>
  );
}

function TestingStepResultRow({ step }: { step: TestingStepResult }) {
  return (
    <div className="rounded-md border border-[var(--aria-line)] p-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <StatusBadge value={step.status} />
        <span className="min-w-0 break-words font-mono text-xs">{step.step_id}</span>
      </div>
      {step.command?.length ? (
        <div className="mt-1 break-words font-mono text-xs text-[var(--aria-ink-muted)]">
          {step.command.join(" ")}
        </div>
      ) : null}
      {step.evidence_refs?.length ? (
        <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
          evidence: {step.evidence_refs.join(", ")}
        </div>
      ) : null}
      {step.provider_analysis ? (
        <div className="mt-1 break-words text-xs">{step.provider_analysis}</div>
      ) : null}
    </div>
  );
}

function TestingList({ label, values }: { label: string; values: string[] }) {
  if (values.length === 0) {
    return null;
  }
  return (
    <div className="space-y-1 text-xs">
      {values.map((value) => (
        <div key={`${label}:${value}`} className="break-words">
          {label}: {value}
        </div>
      ))}
    </div>
  );
}

function EvidencePath({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid min-w-0 grid-cols-[5rem_minmax(0,1fr)] gap-2 text-xs text-[var(--aria-ink-muted)]">
      <span className="font-semibold">{label}</span>
      <span className="min-w-0 break-words font-mono">{value}</span>
    </div>
  );
}

export function ReviewPanel() {
  const codeReviews = useCodingWorkspaceStore((state) => state.codeReviewReports);
  const internalReview = useCodingWorkspaceStore((state) => state.internalPrReview);
  if (codeReviews.length === 0 && !internalReview) {
    return <div className="text-[var(--aria-ink-muted)]">暂无审查报告</div>;
  }
  return (
    <div className="space-y-3">
      {codeReviews.map((report) => (
        <ReviewReportCard
          key={report.id}
          title={`Code Review #${report.round}`}
          report={report}
        />
      ))}
      {internalReview ? (
        <ReviewReportCard title="Internal PR Review" report={internalReview} />
      ) : null}
    </div>
  );
}

function ReviewReportCard({
  title,
  report,
}: {
  title: string;
  report: CodeReviewReport | InternalPrReview;
}) {
  return (
    <div className="rounded-md border border-[var(--aria-line)] p-2">
      <div className="flex items-center justify-between gap-2">
        <div className="text-xs font-semibold">{title}</div>
        <StatusBadge value={report.verdict} />
      </div>
      <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">{report.summary}</div>
      {isInternalPrReview(report) ? (
        <InternalPrReviewDetails review={report} />
      ) : null}
      {report.findings.length > 0 ? (
        <div className="mt-2 space-y-2">
          {report.findings.map((finding, index) => (
            <ReviewFindingItem key={`${finding.file_path ?? "global"}-${finding.line ?? index}-${index}`} finding={finding} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function InternalPrReviewDetails({ review }: { review: InternalPrReview }) {
  return (
    <div className="mt-2 space-y-2 text-xs">
      {review.impact_scope.length > 0 ? (
        <div>
          <div className="font-semibold text-[var(--aria-ink)]">影响范围</div>
          <div className="mt-1 flex flex-wrap gap-1">
            {review.impact_scope.map((scope) => (
              <span
                key={scope}
                className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-1.5 py-0.5 font-mono text-[var(--aria-ink-muted)]"
              >
                {scope}
              </span>
            ))}
          </div>
        </div>
      ) : null}
      {review.pr_description ? (
        <div>
          <div className="font-semibold text-[var(--aria-ink)]">PR description</div>
          <div className="mt-1 whitespace-pre-wrap text-[var(--aria-ink-muted)]">
            {review.pr_description}
          </div>
        </div>
      ) : null}
      {review.commit_message_suggestion ? (
        <div>
          <div className="font-semibold text-[var(--aria-ink)]">commit message</div>
          <div className="mt-1 font-mono text-[var(--aria-ink-muted)]">
            {review.commit_message_suggestion}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function ReviewFindingItem({ finding }: { finding: ReviewFinding }) {
  return (
    <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <span className={["rounded border px-1.5 py-0.5 font-semibold", severityClass(finding.severity)].join(" ")}>
          {finding.severity}
        </span>
        <span className="truncate font-mono text-[var(--aria-ink-muted)]">
          {findingLocation(finding)}
        </span>
      </div>
      <div className="mt-1 text-[var(--aria-ink)]">{finding.message}</div>
      {finding.required_action ? (
        <div className="mt-1 text-[var(--aria-ink-muted)]">{finding.required_action}</div>
      ) : null}
    </div>
  );
}

function isInternalPrReview(report: CodeReviewReport | InternalPrReview): report is InternalPrReview {
  return "review_request_id" in report;
}

function severityClass(severity: ReviewFinding["severity"]) {
  switch (severity) {
    case "error":
      return "border-red-200 bg-red-50 text-red-700";
    case "warning":
      return "border-amber-200 bg-amber-50 text-amber-700";
    case "info":
      return "border-sky-200 bg-sky-50 text-sky-700";
  }
}

function findingLocation(finding: ReviewFinding) {
  if (!finding.file_path) return "global";
  return finding.line ? `${finding.file_path}:${finding.line}` : finding.file_path;
}

export function GitPanel() {
  const store = useCodingWorkspaceStore();
  const request = store.reviewRequest;
  return (
    <div className="space-y-3 text-xs">
      <dl className="space-y-2">
        <InfoRow label="base" value={store.baseBranch ?? request?.base_branch ?? "-"} />
        <InfoRow label="branch" value={store.branchName ?? request?.branch_name ?? "-"} />
        <InfoRow label="commit" value={store.headCommit ?? request?.commit_sha ?? "-"} />
        <InfoRow label="remote" value={store.pushedRemote ?? request?.remote ?? "-"} />
        <InfoRow label="push" value={request?.push_status ?? "-"} />
        <InfoRow label="request" value={request?.id ?? "-"} />
      </dl>
      {request?.external_url ? (
        <a
          href={request.external_url}
          target="_blank"
          rel="noreferrer"
          className="block truncate font-mono text-[var(--aria-primary)] hover:underline"
        >
          {request.external_url}
        </a>
      ) : null}
      {request?.manual_instructions.length ? (
        <ol className="space-y-1 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2">
          {request.manual_instructions.map((instruction, index) => (
            <li key={`${instruction}-${index}`} className="text-[var(--aria-ink-muted)]">
              {instruction}
            </li>
          ))}
        </ol>
      ) : null}
    </div>
  );
}

export function LogsPanel() {
  const logs = useCodingWorkspaceStore((state) => state.logs);
  if (logs.length === 0) return <div className="text-[var(--aria-ink-muted)]">暂无日志</div>;
  return (
    <div className="space-y-2">
      {logs.map((log) => (
        <div key={log.id} className="text-xs">
          {log.timestamp} · {log.message}
        </div>
      ))}
    </div>
  );
}

export function PrepareExecutionPlanPanel({
  attemptId,
  plan,
  requireConfirm,
  onError,
}: {
  attemptId: string;
  plan: WorkItemExecutionPlan;
  requireConfirm: boolean;
  onError: (error: string | null) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [changeNote, setChangeNote] = useState("");
  const showActions = requireConfirm && plan.status !== "confirmed";

  async function handleConfirm() {
    setBusy(true);
    onError(null);
    try {
      const updated = await confirmWorkItemExecutionPlan(attemptId);
      useCodingWorkspaceStore.setState({ workItemExecutionPlan: updated });
    } catch (reason) {
      onError(errorMessage(reason, "确认执行计划失败"));
    } finally {
      setBusy(false);
    }
  }

  async function handleRequestChange() {
    const note = changeNote.trim();
    if (!note) {
      onError("请填写修改说明");
      return;
    }
    setBusy(true);
    onError(null);
    try {
      const updated = await requestWorkItemExecutionPlanChange(attemptId, { note });
      useCodingWorkspaceStore.setState({ workItemExecutionPlan: updated });
      setChangeNote("");
    } catch (reason) {
      onError(errorMessage(reason, "请求修改执行计划失败"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2">
      <div className="flex items-center justify-between">
        <h3 className="text-xs font-semibold uppercase text-[var(--aria-ink)]">执行计划</h3>
        <span className="text-xs text-[var(--aria-ink-muted)]">{plan.status}</span>
      </div>
      <div className="mt-1 text-sm font-semibold text-[var(--aria-ink)]">{plan.goal}</div>
      <dl className="mt-2 grid gap-1 text-xs">
        <InfoRow label="允许写入" value={plan.allowed_write_scopes.join(", ")} />
        {plan.forbidden_write_scopes.length > 0 ? (
          <InfoRow label="禁止写入" value={plan.forbidden_write_scopes.join(", ")} />
        ) : null}
        {plan.dependency_handoffs.length > 0 ? (
          <div className="grid gap-1">
            <dt className="text-[var(--aria-ink-muted)]">依赖交接</dt>
            {plan.dependency_handoffs.map((handoff) => (
              <dd key={handoff.work_item_id} className="min-w-0 break-words font-mono">
                <span>{handoff.work_item_id}</span>
                {handoff.summary ? (
                  <span className="ml-2 text-[var(--aria-ink)]">{handoff.summary}</span>
                ) : null}
                {handoff.commit_sha ? (
                  <span className="ml-2 text-[var(--aria-ink-muted)]">{handoff.commit_sha}</span>
                ) : null}
              </dd>
            ))}
          </div>
        ) : null}
        <InfoRow label="验证计划" value={plan.verification_plan_ref} />
        <InfoRow label="验证摘要" value={plan.verification_summary} />
        {plan.risk_notes.length > 0 ? (
          <InfoRow label="风险说明" value={plan.risk_notes.join("; ")} />
        ) : null}
      </dl>
      {showActions ? (
        <div className="mt-3 grid gap-2">
          <textarea
            aria-label="修改说明"
            value={changeNote}
            onChange={(event) => {
              setChangeNote(event.target.value);
              onError(null);
            }}
            rows={2}
            placeholder="请求修改的原因或补充说明"
            className="min-h-14 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-xs text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)]"
          />
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => void handleConfirm()}
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
            >
              <Check className="h-3.5 w-3.5" />
              确认执行计划
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void handleRequestChange()}
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
            >
              请求修改
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[4rem_minmax(0,1fr)] gap-2">
      <dt className="text-[var(--aria-ink-muted)]">{label}</dt>
      <dd className="truncate font-mono">{value}</dd>
    </div>
  );
}

export function StatusBadge({ value }: { value: string }) {
  return (
    <span className="inline-flex h-6 items-center rounded bg-[var(--aria-panel-subtle)] px-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
      {value}
    </span>
  );
}
