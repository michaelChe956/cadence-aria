import { useRef, useState, type FormEvent } from "react";

export type CreateProjectPayload = {
  name: string;
  description: string | null;
};

export function CreateProjectDialog({
  onCreate,
  onClose,
}: {
  onCreate: (payload: CreateProjectPayload) => Promise<void> | void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submittingRef.current) {
      return;
    }

    const trimmedName = name.trim();
    if (!trimmedName) {
      setError("请输入 Project 名称");
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      await onCreate({
        name: trimmedName,
        description: description.trim() ? description.trim() : null,
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "创建 Project 失败");
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4">
      <form
        role="dialog"
        aria-label="新建 Project"
        aria-modal="true"
        onSubmit={handleSubmit}
        className="w-full max-w-md rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-base font-semibold text-[var(--aria-ink)]">新建 Project</h2>
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
            Project 名称
            <input
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setError(null);
              }}
              className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
            />
          </label>
          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            Project 描述
            <textarea
              value={description}
              onChange={(event) => {
                setDescription(event.target.value);
                setError(null);
              }}
              className="mt-1 block min-h-20 w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
            />
          </label>
          {error ? (
            <p role="alert" className="text-sm font-semibold text-[var(--aria-danger)]">
              {error}
            </p>
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
            创建 Project
          </button>
        </div>
      </form>
    </div>
  );
}
