// Task 9：「知识库更新」弹窗：变更包列表 + 门禁状态 + 新建变更包 + 执行更新。
// 消费 Task 8 的 API 客户端（listKnowledgeBaseChangePackages /
// createKnowledgeBaseChangePackage）；执行更新本身经卡片 onStart 上抛，
// 由宿主调 startKnowledgeBaseInitialization。
import { useEffect, useState } from "react";
import { LoaderCircle, TriangleAlert, X } from "lucide-react";
import {
  createKnowledgeBaseChangePackage,
  listKnowledgeBaseChangePackages,
} from "../../api/knowledge-base";
import type { KnowledgeBaseChangePackageDto } from "../../api/types/knowledge-base";

/**
 * 「执行更新」的二次确认文案：必须写明三件不可逆/易误解的事实——
 * ①会真实调用 provider；②会消耗额度；③会把结果写入聚合根目录。
 */
const UPDATE_CONFIRM_TEXT =
  "知识库更新会真实调用 provider、消耗额度，并把更新结果写入聚合根目录。确认执行更新？";

export type KnowledgeBaseUpdateDialogProps = {
  projectId: string;
  logicalCodebaseId: string;
  open: boolean;
  onClose: () => void;
  onStart: (kind: "update", packagePath?: string) => void;
};

export function KnowledgeBaseUpdateDialog({
  projectId,
  logicalCodebaseId,
  open,
  onClose,
  onStart,
}: KnowledgeBaseUpdateDialogProps) {
  const [packages, setPackages] = useState<KnowledgeBaseChangePackageDto[] | null>(
    null,
  );
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [identifier, setIdentifier] = useState("");
  const [creating, setCreating] = useState(false);
  const [createResult, setCreateResult] = useState<string | null>(null);
  const [createError, setCreateError] = useState<string | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  // 打开时重置本地态（关闭→打开切换才触发；新建成功后的刷新不在此列，
  // 否则提示文案会被随后的列表刷新立即清掉）。
  useEffect(() => {
    if (!open) {
      return;
    }
    setSelectedPath(null);
    setLoadError(null);
    setCreateError(null);
    setCreateResult(null);
  }, [open]);

  // 打开（或新建成功触发刷新）时拉取变更包列表；关闭态不发请求。
  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    setLoading(true);
    listKnowledgeBaseChangePackages(projectId, logicalCodebaseId)
      .then((items) => {
        if (!cancelled) {
          setPackages(Array.isArray(items) ? items : []);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setLoadError(error instanceof Error ? error.message : String(error));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [open, projectId, logicalCodebaseId, reloadTick]);

  // Esc 关闭（与 Provider 管理抽屉同一交互惯例）。
  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [open, onClose]);

  if (!open) {
    return null;
  }

  const selected =
    packages?.find((item) => item.path === selectedPath) ?? null;
  // 门禁：选中门禁通过（问题数为 0）的变更包才能执行更新。
  const executeDisabled = selected === null || selected.violation_count > 0;

  function handleExecute() {
    if (selected === null || executeDisabled) {
      return;
    }
    if (!window.confirm(UPDATE_CONFIRM_TEXT)) {
      return;
    }
    onStart("update", selected.path);
  }

  async function handleCreate() {
    const trimmed = identifier.trim();
    if (trimmed === "" || creating) {
      return;
    }
    setCreating(true);
    setCreateError(null);
    setCreateResult(null);
    try {
      const created = await createKnowledgeBaseChangePackage(
        projectId,
        logicalCodebaseId,
        trimmed,
      );
      setCreateResult(
        `已创建变更包 ${created.path}：请前往知识库详情页填写内容，门禁通过后再执行更新。`,
      );
      setIdentifier("");
      setSelectedPath(created.path);
      setReloadTick((tick) => tick + 1);
    } catch (error: unknown) {
      setCreateError(error instanceof Error ? error.message : String(error));
    } finally {
      setCreating(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/40 p-4"
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="知识库更新"
        data-testid="knowledge-base-update-dialog"
        className="max-h-[80vh] w-full max-w-xl overflow-y-auto rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-lg"
      >
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-sm font-semibold text-[var(--aria-ink)]">
            知识库更新
          </h3>
          <button
            type="button"
            aria-label="关闭"
            onClick={onClose}
            className="rounded p-1 text-[var(--aria-ink-muted)] hover:text-[var(--aria-ink)]"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>
        <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
          仅门禁通过（问题数为 0）的变更包可执行更新；执行会真实调用 provider 并写入聚合根目录。
        </p>

        <div className="mt-3">
          <p className="text-xs font-semibold text-[var(--aria-ink)]">变更包列表</p>
          {loading ? (
            <p
              className="mt-2 flex items-center gap-2 text-xs text-[var(--aria-ink-muted)]"
              data-testid="knowledge-base-packages-loading"
            >
              <LoaderCircle className="h-4 w-4 animate-spin" aria-hidden="true" />
              正在加载变更包…
            </p>
          ) : null}
          {!loading && loadError !== null ? (
            <p
              role="alert"
              className="mt-2 flex items-center gap-2 rounded border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-2 py-1 text-xs text-[var(--aria-danger)]"
            >
              <TriangleAlert className="h-4 w-4 shrink-0" aria-hidden="true" />
              变更包列表加载失败：{loadError}
            </p>
          ) : null}
          {!loading && loadError === null && packages !== null && packages.length === 0 ? (
            <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
              当前没有变更包，可先在下方新建。
            </p>
          ) : null}
          {!loading && packages !== null && packages.length > 0 ? (
            <ul className="mt-2 grid gap-1.5" data-testid="knowledge-base-package-list">
              {packages.map((item) => (
                <li key={item.id}>
                  <label
                    data-testid={`knowledge-base-package-${item.path}`}
                    className="flex flex-wrap items-center gap-2 rounded border border-[var(--aria-line)] px-2 py-1.5 text-xs"
                  >
                    <input
                      type="radio"
                      name="knowledge-base-update-package"
                      checked={selectedPath === item.path}
                      onChange={() => {
                        setSelectedPath(item.path);
                      }}
                    />
                    <span className="font-mono text-[var(--aria-ink)]">{item.path}</span>
                    <span className="text-[var(--aria-ink-muted)]">
                      问题数：{item.violation_count}
                    </span>
                    {item.violation_count > 0 ? (
                      <span className="font-semibold text-[var(--aria-danger)]">
                        门禁未通过
                      </span>
                    ) : (
                      <span className="font-semibold text-emerald-600">门禁通过</span>
                    )}
                  </label>
                  {selectedPath === item.path && item.violations.length > 0 ? (
                    <ul className="mt-1 grid gap-0.5 pl-6 text-[11px] text-[var(--aria-danger)]">
                      {item.violations.map((violation, index) => (
                        <li key={`${item.id}-${violation.code}-${index}`}>
                          {violation.document !== null ? `${violation.document} · ` : ""}
                          {violation.field !== null ? `${violation.field} · ` : ""}
                          {violation.detail}
                        </li>
                      ))}
                    </ul>
                  ) : null}
                </li>
              ))}
            </ul>
          ) : null}
        </div>

        <div className="mt-4 border-t border-[var(--aria-line)] pt-3">
          <p className="text-xs font-semibold text-[var(--aria-ink)]">新建变更包</p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <input
              type="text"
              value={identifier}
              onChange={(event) => {
                setIdentifier(event.target.value);
              }}
              placeholder="变更包标识（如 add-login-feature）"
              data-testid="knowledge-base-new-package-identifier"
              className="h-8 min-w-56 flex-1 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 text-xs text-[var(--aria-ink)]"
            />
            <button
              type="button"
              disabled={creating || identifier.trim() === ""}
              onClick={() => {
                void handleCreate();
              }}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              {creating ? (
                <LoaderCircle
                  className="mr-1 h-4 w-4 animate-spin"
                  aria-hidden="true"
                />
              ) : null}
              新建变更包
            </button>
          </div>
          {createResult !== null ? (
            <p
              data-testid="knowledge-base-create-package-result"
              className="mt-2 rounded border border-emerald-200 bg-emerald-50 px-2 py-1 text-xs text-emerald-700"
            >
              {createResult}
            </p>
          ) : null}
          {createError !== null ? (
            <p
              role="alert"
              className="mt-2 rounded border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-2 py-1 text-xs text-[var(--aria-danger)]"
            >
              新建变更包失败：{createError}
            </p>
          ) : null}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)]"
          >
            关闭
          </button>
          <button
            type="button"
            disabled={executeDisabled}
            onClick={handleExecute}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            执行更新
          </button>
        </div>
      </div>
    </div>
  );
}
