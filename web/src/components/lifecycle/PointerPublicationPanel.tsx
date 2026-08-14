import { GitBranch, TriangleAlert } from "lucide-react";
import type {
  PointerPublicationDto,
  PointerPublicationEntryDto,
  PointerPublicationEntryState,
  PointerPublicationStatus,
} from "../../api/types";

const STATUS_LABELS: Record<PointerPublicationStatus, string> = {
  in_progress: "发布中",
  completed_all: "全部发布完成",
  completed_partial: "部分发布完成",
  revoked: "已撤回",
};

const STATUS_BADGE_CLASSES: Record<PointerPublicationStatus, string> = {
  in_progress:
    "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
  completed_all:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  completed_partial:
    "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
  revoked:
    "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
};

const ENTRY_STATE_LABELS: Record<PointerPublicationEntryState, string> = {
  pending: "等待中",
  skipped: "已跳过",
  conflict: "需人工处理",
  committed: "已提交",
  pushed: "已推送",
  review_created: "已创建 Review",
  failed: "推送失败",
  revoked: "已撤回",
};

export type PointerPublicationPanelProps = {
  publication: PointerPublicationDto | null;
  busy?: boolean;
  onPublishFull: () => void;
  onPublishIncremental: () => void;
  onRetryRepo: (memberRepoId: string) => void;
  onRevoke: () => void;
};

export function PointerPublicationPanel({
  publication,
  busy = false,
  onPublishFull,
  onPublishIncremental,
  onRetryRepo,
  onRevoke,
}: PointerPublicationPanelProps) {
  const inProgress = publication?.status === "in_progress";
  const revoked = publication?.status === "revoked";
  const partial = publication?.status === "completed_partial";

  return (
    <section
      data-testid="pointer-publication-panel"
      className="border-b border-[var(--aria-line)] px-4 py-3"
    >
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-[var(--aria-ink)]">
            指针发布
          </span>
          {publication ? (
            <span
              data-testid="pointer-publication-badge"
              data-status={publication.status}
              className={`inline-flex items-center rounded border px-2 py-1 text-xs font-semibold ${STATUS_BADGE_CLASSES[publication.status]}`}
            >
              {STATUS_LABELS[publication.status]}
            </span>
          ) : null}
        </div>
        {publication && !revoked ? (
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              disabled={inProgress || busy}
              onClick={onPublishFull}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              全量发布
            </button>
            <button
              type="button"
              disabled={inProgress || busy}
              onClick={onPublishIncremental}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              增量发布
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={handleRevokeClick}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-danger)] hover:border-[var(--aria-danger)]"
            >
              撤回
            </button>
          </div>
        ) : null}
      </div>

      {!publication ? (
        <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] p-4">
          <p className="text-sm text-[var(--aria-ink-muted)]">
            尚无指针发布记录。注册成员代码库后，可发起全量发布或增量发布。
          </p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={onPublishFull}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              全量发布
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={onPublishIncremental}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              增量发布
            </button>
          </div>
        </div>
      ) : partial ? (
        <div
          data-testid="pointer-publication-partial-warning"
          className="mb-2 flex items-center gap-2 rounded-md border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs text-[var(--aria-warning)]"
        >
          <TriangleAlert className="h-4 w-4 shrink-0" />
          部分成员发布失败或冲突，需人工处理后重试。
        </div>
      ) : null}

      {publication && publication.entries.length > 0 ? (
        <div className="space-y-2">
          {publication.entries.map((entry) => (
            <PointerPublicationEntryRow
              key={entry.member_repo_id}
              entry={entry}
              onRetryRepo={onRetryRepo}
            />
          ))}
        </div>
      ) : null}

      {publication && publication.entries.length === 0 && !revoked ? (
        <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 text-sm text-[var(--aria-ink-muted)]">
          本次发布没有成员条目。
        </div>
      ) : null}
    </section>
  );

  function handleRevokeClick() {
    if (
      window.confirm(
        "撤回将删除已推送的远端指针分支并标记 ReviewRequest 为已撤回，且无法撤销。确定撤回？",
      )
    ) {
      onRevoke();
    }
  }
}

function PointerPublicationEntryRow({
  entry,
  onRetryRepo,
}: {
  entry: PointerPublicationEntryDto;
  onRetryRepo: (memberRepoId: string) => void;
}) {
  const failed = entry.state === "failed";
  const conflict = entry.state === "conflict";
  const done =
    entry.state === "pushed" || entry.state === "review_created";

  return (
    <div
      data-testid="pointer-publication-entry-row"
      data-state={entry.state}
      className={`rounded-md border px-2 py-2 text-xs ${
        failed || conflict
          ? "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)]"
          : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
      }`}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono font-semibold text-[var(--aria-ink)]">
          {entry.member_repo_id}
        </span>
        <span
          className={
            failed
              ? "font-semibold text-[var(--aria-danger)]"
              : conflict
                ? "font-semibold text-[var(--aria-warning)]"
                : done
                  ? "font-semibold text-[var(--aria-success)]"
                  : "text-[var(--aria-ink-muted)]"
          }
        >
          {ENTRY_STATE_LABELS[entry.state]}
        </span>
      </div>
      {entry.branch_name || entry.commit_sha ? (
        <div className="mt-1 flex items-center gap-2 text-[var(--aria-ink-muted)]">
          <GitBranch className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">{entry.branch_name ?? "—"}</span>
          {entry.commit_sha ? (
            <span className="shrink-0 font-mono text-[11px]">
              {entry.commit_sha.slice(0, 7)}
            </span>
          ) : null}
        </div>
      ) : null}
      {conflict && entry.conflict_detail ? (
        <div
          data-testid="pointer-publication-conflict-detail"
          className="mt-1 text-[var(--aria-warning)]"
        >
          {entry.conflict_detail}
        </div>
      ) : null}
      {failed && entry.push_error ? (
        <div
          data-testid="pointer-publication-push-error"
          className="mt-1 text-[var(--aria-danger)]"
        >
          {entry.push_error}
        </div>
      ) : null}
      {conflict ? (
        <button
          type="button"
          onClick={() => onRetryRepo(entry.member_repo_id)}
          className="mt-2 inline-flex h-7 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-2.5 text-xs font-semibold text-white"
        >
          人工已解决，重试
        </button>
      ) : null}
      {failed ? (
        <button
          type="button"
          onClick={() => onRetryRepo(entry.member_repo_id)}
          className="mt-2 inline-flex h-7 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-2.5 text-xs font-semibold text-white"
        >
          重试
        </button>
      ) : null}
    </div>
  );
}
