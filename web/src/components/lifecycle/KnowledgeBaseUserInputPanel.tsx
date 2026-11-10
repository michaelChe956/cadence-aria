// Task 6：知识库输入确认区（REQ-KB-15 前端面）。
// 列出 9 份输入文件与三态徽标（未填 / 待确认 / 已确认），支持逐份确认与
// 全部确认；点击某份交给页面在右侧编辑器打开并可直接编辑。
// 确认必须携带「当前读到的 digest」：面板在确认前先读一次文件取服务端
// 摘要，摘要不符（409 knowledge_base_user_input_confirmation_stale）时给出
// 可执行提示并刷新投影。已确认文件被编辑后确认按 digest 自动失效（后端
// 判定），页面把「已确认 → 待确认」的迁移传入 invalidatedPaths，本组件
// 逐份提示「已修改，需重新确认」。
import { useCallback, useState } from "react";
import { ApiRequestError } from "../../api/client";
import {
  confirmAllKnowledgeBaseUserInput,
  confirmKnowledgeBaseUserInputFile,
  readKnowledgeBaseFile,
} from "../../api/knowledge-base";
import type { KnowledgeBaseUserInputSummaryDto } from "../../api/types/knowledge-base";

const CONFIRMATION_STALE_CODE = "knowledge_base_user_input_confirmation_stale";

type Notice = { kind: "error" | "success"; text: string };

/** 确认失败的文案：过期确认（stale）的服务端 message 已是中文可执行文案，原样透出。 */
function describeConfirmError(reason: unknown): string {
  if (reason instanceof ApiRequestError) {
    if (reason.code === CONFIRMATION_STALE_CODE && reason.message) {
      return reason.message;
    }
    return `确认失败：${reason.message}`;
  }
  if (reason instanceof Error) {
    return `确认失败：${reason.message}`;
  }
  return "确认失败：未知错误";
}

function isConfirmationStale(reason: unknown): boolean {
  return (
    reason instanceof ApiRequestError && reason.code === CONFIRMATION_STALE_CODE
  );
}

export function KnowledgeBaseUserInputPanel({
  projectId,
  lcId,
  summary,
  invalidatedPaths,
  selectedPath,
  onOpenFile,
  onSummaryChanged,
}: {
  projectId: string;
  lcId: string;
  summary: KnowledgeBaseUserInputSummaryDto;
  /** 已确认后被编辑（摘要漂移）的路径集合：逐份提示需重新确认。 */
  invalidatedPaths: ReadonlySet<string>;
  selectedPath: string | null;
  onOpenFile: (path: string) => void;
  /** 确认动作落地（成功或确认过期）后请求页面重新拉取汇总投影。 */
  onSummaryChanged: () => void;
}) {
  const [notice, setNotice] = useState<Notice | null>(null);
  const [confirmingPaths, setConfirmingPaths] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [confirmingAll, setConfirmingAll] = useState(false);

  // 待确认数 = 总数 - 已确认数（含未填份额，与后端 not_confirmed 拒绝口径一致）。
  const pendingCount = Math.max(
    0,
    summary.total_count - summary.confirmed_count,
  );
  const hasPendingFile = summary.files.some((file) => file.pending);

  const confirmOne = useCallback(
    async (path: string) => {
      setNotice(null);
      setConfirmingPaths((prev) => new Set(prev).add(path));
      try {
        // 确认请求携带当前读到的 digest：先读文件拿服务端当前摘要。
        const file = await readKnowledgeBaseFile(projectId, lcId, path);
        await confirmKnowledgeBaseUserInputFile(
          projectId,
          lcId,
          path,
          file.digest,
        );
        setNotice({ kind: "success", text: `已确认 ${path}` });
        onSummaryChanged();
      } catch (reason) {
        setNotice({ kind: "error", text: describeConfirmError(reason) });
        // 过期 ⇒ 文件内容已变化：刷新汇总投影，让徽标回到最新事实。
        if (isConfirmationStale(reason)) {
          onSummaryChanged();
        }
      } finally {
        setConfirmingPaths((prev) => {
          const next = new Set(prev);
          next.delete(path);
          return next;
        });
      }
    },
    [projectId, lcId, onSummaryChanged],
  );

  const confirmAll = useCallback(async () => {
    // 批量只带「待确认」文件：已确认的无需重复；未填的确认无意义
    //（服务端投影的 confirmed 只计已填写文件）。
    const pending = summary.files.filter((file) => file.pending);
    if (pending.length === 0 || confirmingAll) {
      return;
    }
    setNotice(null);
    setConfirmingAll(true);
    try {
      const items: { path: string; digest: string }[] = [];
      for (const file of pending) {
        const current = await readKnowledgeBaseFile(projectId, lcId, file.path);
        items.push({ path: file.path, digest: current.digest });
      }
      await confirmAllKnowledgeBaseUserInput(projectId, lcId, items);
      setNotice({ kind: "success", text: `已确认 ${items.length} 份输入文件` });
      onSummaryChanged();
    } catch (reason) {
      setNotice({ kind: "error", text: describeConfirmError(reason) });
      if (isConfirmationStale(reason)) {
        onSummaryChanged();
      }
    } finally {
      setConfirmingAll(false);
    }
  }, [projectId, lcId, summary, confirmingAll, onSummaryChanged]);

  return (
    <section
      data-testid="kb-user-input-panel"
      aria-label="输入确认区"
      className="flex max-h-72 shrink-0 flex-col gap-2 overflow-hidden rounded-md border border-[var(--aria-line)] bg-white p-2"
    >
      <div className="flex shrink-0 items-center justify-between gap-2 px-1">
        <span className="text-xs font-semibold text-[var(--aria-ink)]">
          输入确认
        </span>
        <button
          type="button"
          data-testid="kb-user-input-confirm-all"
          disabled={confirmingAll || !hasPendingFile}
          onClick={() => void confirmAll()}
          className="rounded-md border border-[var(--aria-line)] bg-white px-2 py-1 text-xs font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
        >
          全部确认
        </button>
      </div>
      {/* 未确认时提示体现「待确认 N 份」；全部确认后提示可执行初始化。 */}
      <p
        data-testid="kb-user-input-summary"
        className="shrink-0 px-1 text-xs text-[var(--aria-ink-muted)]"
      >
        {pendingCount > 0
          ? `已确认 ${summary.confirmed_count}/${summary.total_count}，待确认 ${pendingCount} 份：确认全部输入后才能执行【知识库初始化】`
          : `已确认 ${summary.confirmed_count}/${summary.total_count}，可执行【知识库初始化】`}
      </p>
      {notice ? (
        <div
          role={notice.kind === "error" ? "alert" : "status"}
          data-testid="kb-user-input-notice"
          data-kind={notice.kind}
          className={
            notice.kind === "error"
              ? "shrink-0 rounded-md border border-red-200 bg-red-50 px-2 py-1.5 text-xs text-red-700"
              : "shrink-0 rounded-md border border-emerald-200 bg-emerald-50 px-2 py-1.5 text-xs text-emerald-700"
          }
        >
          {notice.text}
        </div>
      ) : null}
      <ul className="min-h-0 flex-1 space-y-1 overflow-auto">
        {summary.files.map((file) => {
          const state = file.confirmed
            ? "confirmed"
            : file.filled
              ? "pending"
              : "unfilled";
          const stateText =
            state === "confirmed"
              ? "已确认"
              : state === "pending"
                ? "待确认"
                : "未填";
          return (
            <li key={file.path}>
              <div
                data-testid="kb-user-input-item"
                data-path={file.path}
                data-state={state}
                className="flex items-center gap-1 rounded px-1 py-0.5 hover:bg-[var(--aria-panel-muted)]"
              >
                <button
                  type="button"
                  data-testid="kb-user-input-open"
                  data-path={file.path}
                  title={file.path}
                  aria-current={selectedPath === file.path}
                  onClick={() => onOpenFile(file.path)}
                  className={
                    selectedPath === file.path
                      ? "min-w-0 flex-1 truncate rounded px-1 py-0.5 text-left text-xs font-semibold text-[var(--aria-ink)] bg-[var(--aria-panel-muted)]"
                      : "min-w-0 flex-1 truncate rounded px-1 py-0.5 text-left text-xs text-[var(--aria-ink)]"
                  }
                >
                  {file.path.split("/").at(-1) ?? file.path}
                </button>
                <span
                  data-testid="kb-user-input-badge"
                  data-path={file.path}
                  data-state={state}
                  className={
                    state === "confirmed"
                      ? "shrink-0 rounded border border-emerald-200 bg-emerald-50 px-1.5 py-0.5 text-[11px] text-emerald-700"
                      : state === "pending"
                        ? "shrink-0 rounded border border-amber-200 bg-amber-50 px-1.5 py-0.5 text-[11px] text-amber-800"
                        : "shrink-0 rounded border border-[var(--aria-line)] bg-white px-1.5 py-0.5 text-[11px] text-[var(--aria-ink-muted)]"
                  }
                >
                  {stateText}
                </span>
                {file.pending ? (
                  <button
                    type="button"
                    data-testid="kb-user-input-confirm"
                    data-path={file.path}
                    disabled={confirmingPaths.has(file.path) || confirmingAll}
                    onClick={() => void confirmOne(file.path)}
                    className="shrink-0 rounded border border-[var(--aria-line)] bg-white px-1.5 py-0.5 text-[11px] font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
                  >
                    确认
                  </button>
                ) : null}
              </div>
              {invalidatedPaths.has(file.path) ? (
                <p
                  data-testid="kb-user-input-invalidated"
                  data-path={file.path}
                  className="px-2 pb-0.5 text-[11px] text-amber-800"
                >
                  已修改，需重新确认
                </p>
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
