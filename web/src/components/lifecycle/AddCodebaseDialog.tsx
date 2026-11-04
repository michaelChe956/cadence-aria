import { useState, type FormEvent } from "react";
import { createLogicalCodebase } from "../../api/codebases";
import { LogicalCodebaseLayoutTree } from "./LogicalCodebaseLayoutTree";
import { LogicalCodebaseRegistrationWizard } from "./LogicalCodebaseRegistrationWizard";
import { DialogShell } from "../shared/DialogShell";
import type { LogicalCodebaseDto } from "../../api/types";

function aggregateRootBasename(aggregateRoot: string): string {
  const normalized = aggregateRoot.trim().replace(/\/+$/, "");
  if (!normalized) return "";
  const base = normalized.split(/[\\/]/).pop() ?? "";
  return base;
}

export function AddCodebaseDialog({
  projectId,
  onCreatedLogical,
  onRegistrationCompleted,
  onClose,
}: {
  projectId: string;
  onCreatedLogical: (codebase: LogicalCodebaseDto) => void;
  /** 登记批完成后回调（成员已变化，宿主应刷新并触发冷启动自动推进）；
   *  参数为登记完成的 LC id。创建+登记合并在本弹窗内完成。 */
  onRegistrationCompleted?: (logicalCodebaseId: string) => Promise<void> | void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [nameTouched, setNameTouched] = useState(false);
  const [aggregateRoot, setAggregateRoot] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [createdLogical, setCreatedLogical] = useState<LogicalCodebaseDto | null>(null);

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
      // 合并流：弹窗保持打开，切换到登记阶段并复用同一聚合根（不再弹独立登记向导）。
      setCreatedLogical(codebase);
      onCreatedLogical(codebase);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "创建逻辑代码库失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <DialogShell
      label="添加代码库"
      onClose={onClose}
      closeLabel="关闭"
      closeDisabled={busy}
      widthClass={createdLogical ? "max-w-2xl" : "max-w-lg"}
    >
      {createdLogical ? (
        <div>
          <p className="mb-3 text-sm text-[var(--aria-ink-muted)]">
            逻辑代码库「{createdLogical.name}」已创建，继续登记成员（聚合根已复用）。
          </p>
          <LogicalCodebaseRegistrationWizard
            projectId={projectId}
            logicalCodebaseId={createdLogical.id}
            initialAggregateRoot={createdLogical.aggregate_root}
            autoStart
            embedded
            onCompleted={onRegistrationCompleted}
          />
        </div>
      ) : (
        <form onSubmit={handleCreateLogical} className="mt-4 space-y-3">
          <p className="text-sm text-[var(--aria-ink-muted)]">
            以聚合根目录组织多个成员仓库，创建后在本弹窗内登记成员（自动发现）。
          </p>
          {/* 目录布局说明：复用同一处布局文案源（`LogicalCodebaseLayoutTree`，
              与登记向导 / 冷启动卡同一组件），不另抄一份文案。
              REQ-LAYOUT-02：多仓创建/登记入口 SHALL 在显著位置给出目录树示意。 */}
          <LogicalCodebaseLayoutTree />
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
    </DialogShell>
  );
}
