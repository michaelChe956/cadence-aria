import { useMemo, useRef, useState, type FormEvent } from "react";
import {
  preflightLogicalCodebaseRegistration,
  resumeLogicalCodebaseRegistration,
  submitLogicalCodebaseRegistration,
} from "../../api/logical-codebase-registration";
import type {
  RegistrationBatchDto,
  RegistrationPreflightItemDto,
  RegistrationPreflightResponse,
} from "../../api/types";

const PREFLIGHT_CLASSES = [
  "eligible",
  "non_git",
  "duplicate",
  "nested",
  "needs_attention",
  "missing",
  "outside_root",
] as const;

function formatError(reason: unknown, fallback: string): string {
  return reason instanceof Error ? reason.message : fallback;
}

function displayReason(item: RegistrationPreflightItemDto): string {
  return item.reason ? `（${item.reason}）` : "";
}

export function LogicalCodebaseRegistrationWizard({
  projectId,
  onCompleted,
  onClose,
}: {
  projectId: string;
  onCompleted: () => Promise<void> | void;
  onClose: () => void;
}) {
  const [aggregateRoot, setAggregateRoot] = useState("");
  const [candidatePaths, setCandidatePaths] = useState("");
  const [preflight, setPreflight] = useState<RegistrationPreflightResponse | null>(null);
  const [confirmedAttentionPaths, setConfirmedAttentionPaths] = useState<Set<string>>(
    () => new Set(),
  );
  const [batch, setBatch] = useState<RegistrationBatchDto | null>(null);
  const [busyAction, setBusyAction] = useState<"preflight" | "submit" | "resume" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busyRef = useRef(false);

  const paths = useMemo(
    () => candidatePaths.split(/\r?\n/).map((path) => path.trim()).filter(Boolean),
    [candidatePaths],
  );
  const confirmedPaths = useMemo(() => {
    if (!preflight) return [];
    return preflight.items
      .filter(
        (item) =>
          item.class === "eligible" ||
          (item.class === "needs_attention" && confirmedAttentionPaths.has(item.path)),
      )
      .map((item) => item.path);
  }, [confirmedAttentionPaths, preflight]);

  async function handlePreflight(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busyRef.current) return;
    const root = aggregateRoot.trim();
    if (!root) {
      setError("请输入聚合根目录");
      return;
    }
    busyRef.current = true;
    setBusyAction("preflight");
    setError(null);
    setBatch(null);
    try {
      const result = await preflightLogicalCodebaseRegistration(projectId, {
        aggregate_root: root,
        candidate_paths: paths,
      });
      setPreflight(result);
      setConfirmedAttentionPaths(new Set());
    } catch (reason) {
      setError(formatError(reason, "预检失败"));
    } finally {
      busyRef.current = false;
      setBusyAction(null);
    }
  }

  async function handleSubmit() {
    if (busyRef.current || !preflight) return;
    if (
      preflight.items.some(
        (item) => item.class === "needs_attention" && !confirmedAttentionPaths.has(item.path),
      )
    ) {
      setError("请确认需要关注的成员后再提交登记");
      return;
    }
    if (confirmedPaths.length === 0) {
      setError("没有可提交的成员");
      return;
    }
    busyRef.current = true;
    setBusyAction("submit");
    setError(null);
    try {
      const result = await submitLogicalCodebaseRegistration(projectId, {
        aggregate_root: aggregateRoot.trim(),
        preflight_id: preflight.preflight_id,
        confirmed_paths: confirmedPaths,
      });
      setBatch(result);
      if (result.status === "completed") {
        await onCompleted();
      }
    } catch (reason) {
      setError(formatError(reason, "提交登记失败"));
    } finally {
      busyRef.current = false;
      setBusyAction(null);
    }
  }

  async function handleResume() {
    if (busyRef.current || !batch) return;
    busyRef.current = true;
    setBusyAction("resume");
    setError(null);
    try {
      const result = await resumeLogicalCodebaseRegistration(projectId, batch.batch_id);
      setBatch(result);
      if (result.status === "completed") {
        await onCompleted();
      }
    } catch (reason) {
      setError(formatError(reason, "恢复登记失败"));
    } finally {
      busyRef.current = false;
      setBusyAction(null);
    }
  }

  const submitting = busyAction === "submit";
  const canSubmit = Boolean(preflight) && !batch && !busyAction;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4">
      <section
        role="dialog"
        aria-modal="true"
        aria-label="登记成员"
        className="max-h-[90vh] w-full max-w-2xl overflow-auto rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-base font-semibold text-[var(--aria-ink)]">登记成员</h2>
          <button
            type="button"
            onClick={onClose}
            disabled={Boolean(busyAction)}
            className="rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)] disabled:opacity-60"
          >
            关闭
          </button>
        </div>

        <form onSubmit={handlePreflight} className="space-y-3">
          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            聚合根目录
            <input
              aria-label="聚合根目录"
              value={aggregateRoot}
              onChange={(event) => setAggregateRoot(event.target.value)}
              disabled={Boolean(busyAction) || Boolean(preflight)}
              className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)] disabled:opacity-60"
            />
          </label>
          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            候选成员路径
            <textarea
              aria-label="候选成员路径"
              value={candidatePaths}
              onChange={(event) => setCandidatePaths(event.target.value)}
              disabled={Boolean(busyAction) || Boolean(preflight)}
              placeholder="每行一个路径（可选）"
              className="mt-1 block min-h-20 w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 font-mono text-sm font-normal text-[var(--aria-ink)] disabled:opacity-60"
            />
          </label>
          {!preflight ? (
            <button
              type="submit"
              disabled={Boolean(busyAction)}
              className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
            >
              {busyAction === "preflight" ? "预检中…" : "执行预检"}
            </button>
          ) : null}
        </form>

        {preflight ? (
          <div className="mt-4 space-y-3">
            <div className="flex items-center justify-between gap-2">
              <h3 className="text-sm font-semibold text-[var(--aria-ink)]">预检分类</h3>
              <button
                type="button"
                disabled={Boolean(busyAction) || Boolean(batch)}
                onClick={() => {
                  setPreflight(null);
                  setConfirmedAttentionPaths(new Set());
                }}
                className="rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)] disabled:opacity-60"
              >
                重新预检
              </button>
            </div>
            <div className="space-y-2">
              {PREFLIGHT_CLASSES.map((classification) => {
                const items = preflight.items.filter((item) => item.class === classification);
                return (
                  <section
                    key={classification}
                    aria-label={`分类 ${classification}`}
                    className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
                  >
                    <h4 className="text-xs font-semibold text-[var(--aria-ink)]">{classification}</h4>
                    {items.length > 0 ? (
                      <ul className="mt-1 space-y-1">
                        {items.map((item) => (
                          <li key={item.path} className="text-xs text-[var(--aria-ink-muted)]">
                            {item.class === "needs_attention" ? (
                              <label className="inline-flex items-center gap-2">
                                <input
                                  type="checkbox"
                                  aria-label={`确认 ${item.path}${displayReason(item)}`}
                                  checked={confirmedAttentionPaths.has(item.path)}
                                  disabled={Boolean(busyAction) || Boolean(batch)}
                                  onChange={(event) => {
                                    setConfirmedAttentionPaths((current) => {
                                      const next = new Set(current);
                                      if (event.target.checked) next.add(item.path);
                                      else next.delete(item.path);
                                      return next;
                                    });
                                  }}
                                />
                                确认 {item.path}{displayReason(item)}
                              </label>
                            ) : (
                              <span>{item.path}{displayReason(item)}</span>
                            )}
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">无</p>
                    )}
                  </section>
                );
              })}
            </div>
            {!batch ? (
              <button
                type="button"
                disabled={!canSubmit || submitting}
                onClick={() => void handleSubmit()}
                aria-label="提交登记"
                aria-busy={submitting}
                className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
              >
                {submitting ? "⏳ " : ""}提交登记
              </button>
            ) : null}
          </div>
        ) : null}

        {batch ? (
          <section aria-label="登记结果" className="mt-4 space-y-3">
            <h3 className="text-sm font-semibold text-[var(--aria-ink)]">登记结果</h3>
            <p className="font-mono text-sm text-[var(--aria-ink)]">{batch.status}</p>
            <ul className="space-y-1 text-xs text-[var(--aria-ink-muted)]">
              {batch.items.map((item) => (
                <li key={item.path}>
                  {item.path}: {item.status}{item.failure_reason ? `（${item.failure_reason}）` : ""}
                </li>
              ))}
            </ul>
            {batch.status === "partial_failed" ? (
              <button
                type="button"
                disabled={busyAction === "resume"}
                onClick={() => void handleResume()}
                aria-label="恢复未完成项"
                aria-busy={busyAction === "resume"}
                className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
              >
                {busyAction === "resume" ? "⏳ " : ""}恢复未完成项
              </button>
            ) : null}
          </section>
        ) : null}

        {error ? (
          <p role="alert" className="mt-3 text-sm font-semibold text-[var(--aria-danger)]">
            {error}
          </p>
        ) : null}
      </section>
    </div>
  );
}
