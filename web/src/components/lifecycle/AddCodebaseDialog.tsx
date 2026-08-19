import { useState, type FormEvent } from "react";
import { createLogicalCodebase } from "../../api/codebases";
import type { LogicalCodebaseDto } from "../../api/types";

type AddCodebaseMode = "single_repo" | "logical";

function aggregateRootBasename(aggregateRoot: string): string {
  const normalized = aggregateRoot.trim().replace(/\/+$/, "");
  if (!normalized) return "";
  const base = normalized.split(/[\\/]/).pop() ?? "";
  return base;
}

export function AddCodebaseDialog({
  projectId,
  onChooseSingle,
  onCreatedLogical,
  onClose,
}: {
  projectId: string;
  onChooseSingle: () => void;
  onCreatedLogical: (codebase: LogicalCodebaseDto) => void;
  onClose: () => void;
}) {
  const [mode, setMode] = useState<AddCodebaseMode>("single_repo");
  const [name, setName] = useState("");
  const [nameTouched, setNameTouched] = useState(false);
  const [aggregateRoot, setAggregateRoot] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const derivedName = nameTouched ? name : aggregateRootBasename(aggregateRoot);

  async function handleCreateLogical(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy) return;
    const trimmedName = derivedName.trim();
    const trimmedRoot = aggregateRoot.trim();
    if (!trimmedRoot) {
      setError("请输入聚合根目录");
      return;
    }
    if (!trimmedName) {
      setError("请输入逻辑代码库名称");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const codebase = await createLogicalCodebase(projectId, {
        name: trimmedName,
        aggregate_root: trimmedRoot,
      });
      onCreatedLogical(codebase);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "创建逻辑代码库失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4">
      <section
        role="dialog"
        aria-modal="true"
        aria-label="添加代码库"
        className="w-full max-w-lg rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-base font-semibold text-[var(--aria-ink)]">添加代码库</h2>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)] disabled:opacity-60"
          >
            关闭
          </button>
        </div>

        <fieldset className="space-y-2">
          <legend className="text-sm font-semibold text-[var(--aria-ink)]">模式</legend>
          <label className="flex items-start gap-2 rounded-md border border-[var(--aria-line)] p-2 text-sm">
            <input
              type="radio"
              name="add-codebase-mode"
              checked={mode === "single_repo"}
              onChange={() => setMode("single_repo")}
              disabled={busy}
              className="mt-1"
            />
            <span>
              <span className="font-semibold text-[var(--aria-ink)]">单仓库</span>
              <span className="block text-xs text-[var(--aria-ink-muted)]">
                绑定一个既有 git 仓库，走原有添加流程。
              </span>
            </span>
          </label>
          <label className="flex items-start gap-2 rounded-md border border-[var(--aria-line)] p-2 text-sm">
            <input
              type="radio"
              name="add-codebase-mode"
              checked={mode === "logical"}
              onChange={() => setMode("logical")}
              disabled={busy}
              className="mt-1"
            />
            <span>
              <span className="font-semibold text-[var(--aria-ink)]">多仓库逻辑代码库</span>
              <span className="block text-xs text-[var(--aria-ink-muted)]">
                以聚合根目录组织多个成员仓库，创建后进入登记向导（自动发现）。
              </span>
            </span>
          </label>
        </fieldset>

        {mode === "single_repo" ? (
          <div className="mt-4 flex justify-end">
            <button
              type="button"
              disabled={busy}
              onClick={onChooseSingle}
              className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
            >
              继续添加单仓库
            </button>
          </div>
        ) : (
          <form onSubmit={handleCreateLogical} className="mt-4 space-y-3">
            <label className="block text-sm font-semibold text-[var(--aria-ink)]">
              聚合根目录
              <input
                aria-label="聚合根目录"
                value={aggregateRoot}
                onChange={(event) => setAggregateRoot(event.target.value)}
                disabled={busy}
                placeholder="/repos/monorepo"
                className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 font-mono text-sm font-normal text-[var(--aria-ink)] disabled:opacity-60"
              />
            </label>
            <label className="block text-sm font-semibold text-[var(--aria-ink)]">
              名称
              <input
                aria-label="名称"
                value={derivedName}
                onChange={(event) => {
                  setNameTouched(true);
                  setName(event.target.value);
                }}
                disabled={busy}
                placeholder="默认取聚合根目录名"
                className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)] disabled:opacity-60"
              />
            </label>
            <div className="flex justify-end">
              <button
                type="submit"
                disabled={busy}
                aria-busy={busy}
                className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
              >
                {busy ? "创建中…" : "创建逻辑代码库"}
              </button>
            </div>
          </form>
        )}

        {error ? (
          <p role="alert" className="mt-3 text-sm font-semibold text-[var(--aria-danger)]">
            {error}
          </p>
        ) : null}
      </section>
    </div>
  );
}
