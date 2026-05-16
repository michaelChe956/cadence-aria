import { useState, type FormEvent } from "react";
import type { Repository } from "../../api/types";

export type CreateLifecycleIssuePayload = {
  title: string;
  description: string | null;
  repository_id: string;
};

export function CreateLifecycleIssueDialog({
  repositories,
  onCreate,
  onClose,
}: {
  repositories: Repository[];
  onCreate: (payload: CreateLifecycleIssuePayload) => Promise<void> | void;
  onClose: () => void;
}) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [repositoryId, setRepositoryId] = useState("");
  const [repositoryError, setRepositoryError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!repositoryId) {
      setRepositoryError("请选择代码库");
      return;
    }

    setSubmitting(true);
    setRepositoryError(null);
    try {
      await onCreate({
        title: title.trim(),
        description: description.trim() ? description.trim() : null,
        repository_id: repositoryId,
      });
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4">
      <form
        role="dialog"
        aria-label="新建 Issue"
        aria-modal="true"
        onSubmit={handleSubmit}
        className="w-full max-w-lg rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-base font-semibold text-[var(--aria-ink)]">新建 Issue</h2>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]"
          >
            关闭
          </button>
        </div>
        <div className="space-y-3">
          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            Issue 标题
            <input
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
            />
          </label>
          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            Issue 描述
            <textarea
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              className="mt-1 block min-h-24 w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
            />
          </label>
          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            代码库
            <select
              value={repositoryId}
              aria-invalid={repositoryError ? "true" : undefined}
              onChange={(event) => {
                setRepositoryId(event.target.value);
                setRepositoryError(null);
              }}
              className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
            >
              <option value="">请选择</option>
              {repositories.map((repository) => (
                <option key={repository.repository_id} value={repository.repository_id}>
                  {repository.name} · {repository.repository_id}
                </option>
              ))}
            </select>
          </label>
          {repositoryError ? (
            <p className="text-sm font-semibold text-[var(--aria-danger)]">{repositoryError}</p>
          ) : null}
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink-muted)]"
          >
            取消
          </button>
          <button
            type="submit"
            disabled={submitting}
            className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
          >
            创建 Issue
          </button>
        </div>
      </form>
    </div>
  );
}
