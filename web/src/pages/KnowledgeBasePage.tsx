// Task 10：LC 知识库详情页。
// 左侧文件树（按目录分组）+ 右侧「编辑 / 预览」页签 + 保存按钮。
// 编辑页签用 Task 8 的可写 MonacoEditor，预览页签用公共 MarkdownContent；
// 变更包（user-input/updates/ 下的 CHANGE-* 目录）额外展示 lint 的字段级标记。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ApiRequestError } from "../api/client";
import {
  getKnowledgeBaseStatus,
  lintKnowledgeBaseChangePackage,
  listKnowledgeBaseFiles,
  readKnowledgeBaseFile,
  writeKnowledgeBaseFile,
} from "../api/knowledge-base";
import type {
  KnowledgeBaseChangePackageFieldStatusDto,
  KnowledgeBaseFileNodeDto,
  KnowledgeBaseStatusDto,
} from "../api/types/knowledge-base";
import { MarkdownContent } from "../components/shared/MarkdownContent";
import { MonacoEditor } from "../components/shared/MonacoEditor";
import { useUnloadGuard } from "../hooks/useUnloadGuard";

const CHANGE_PACKAGE_SEGMENT = "/user-input/updates/";
const UNSAVED_MESSAGE = "知识库文档有未保存的修改，离开将丢失这些改动。是否继续？";

type Notice = { kind: "error" | "success"; text: string };

type TreeEntry =
  | { kind: "dir"; name: string; path: string; children: TreeEntry[] }
  | { kind: "file"; name: string; path: string };

export function KnowledgeBasePage({
  projectId,
  lcId,
}: {
  projectId: string;
  lcId: string;
}) {
  const [status, setStatus] = useState<KnowledgeBaseStatusDto | null>(null);
  const [nodes, setNodes] = useState<KnowledgeBaseFileNodeDto[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [savedContent, setSavedContent] = useState("");
  const [draft, setDraft] = useState("");
  const [digest, setDigest] = useState<string | null>(null);
  const [tab, setTab] = useState<"edit" | "preview">("edit");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [saving, setSaving] = useState(false);
  const [lintFields, setLintFields] = useState<KnowledgeBaseChangePackageFieldStatusDto[]>([]);
  // 当前选中的文件路径（同步镜像）：用于丢弃乱序到达的过期读取响应。
  const selectedPathRef = useRef<string | null>(null);

  const dirty = selectedPath !== null && draft !== savedContent;

  useUnloadGuard({ enabled: dirty, message: UNSAVED_MESSAGE });

  const openFile = useCallback(
    async (path: string) => {
      selectedPathRef.current = path;
      setSelectedPath(path);
      setNotice(null);
      // 读取新文件前先清空内容状态：读取失败时不得残留上一文件的内容
      // （否则保存会把旧内容写进新路径），digest 置空同时使「保存」按钮禁用
      // （没有读时摘要不得发出无 baseDigest 的盲覆盖写）。
      setSavedContent("");
      setDraft("");
      setDigest(null);
      setLintFields([]);
      try {
        const file = await readKnowledgeBaseFile(projectId, lcId, path);
        if (selectedPathRef.current !== path) {
          // 已切换到别的文件：本响应过期，直接丢弃（不得覆盖当前文件内容）。
          return;
        }
        setSavedContent(file.content);
        setDraft(file.content);
        setDigest(file.digest);
      } catch (reason) {
        if (selectedPathRef.current !== path) {
          return;
        }
        setNotice({ kind: "error", text: `读取文件失败：${errorText(reason)}` });
        return;
      }
      const packagePath = changePackagePathOf(path);
      if (!packagePath) {
        setLintFields([]);
        return;
      }
      try {
        const lint = await lintKnowledgeBaseChangePackage(projectId, lcId, packagePath);
        if (selectedPathRef.current !== path) {
          return;
        }
        setLintFields(lint.fields);
      } catch {
        if (selectedPathRef.current !== path) {
          return;
        }
        // lint 不可用（后端未就绪 / 网络失败）不得让页面崩：按「无标记」降级。
        setLintFields([]);
      }
    },
    [projectId, lcId],
  );

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      let fileNodes: KnowledgeBaseFileNodeDto[] = [];
      try {
        fileNodes = await listKnowledgeBaseFiles(projectId, lcId);
      } catch (reason) {
        if (!cancelled) {
          setNotice({ kind: "error", text: `加载知识库文件失败：${errorText(reason)}` });
        }
      }
      if (cancelled) {
        return;
      }
      setNodes(fileNodes);
      const first = fileNodes.find((node) => !node.is_dir);
      if (first) {
        await openFile(first.path);
      }
    })();
    void getKnowledgeBaseStatus(projectId, lcId)
      .then((next) => {
        if (!cancelled) {
          setStatus(next);
        }
      })
      .catch(() => {
        // status 不可用不阻塞文件浏览（后端未就绪时的正常降级）。
        if (!cancelled) {
          setStatus(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [projectId, lcId, openFile]);

  const save = useCallback(async () => {
    if (!selectedPath) {
      return;
    }
    if (digest === null) {
      // 没有读时摘要一律不写：否则会发出无 baseDigest 的盲覆盖写。
      setNotice({
        kind: "error",
        text: "文件尚未成功读取，无法保存（缺少读时摘要）。",
      });
      return;
    }
    setSaving(true);
    try {
      const result = await writeKnowledgeBaseFile(
        projectId,
        lcId,
        selectedPath,
        draft,
        digest,
      );
      if (selectedPathRef.current !== selectedPath) {
        // 保存期间已切换到别的文件：不得把旧文件的保存结果落到当前文件上。
        return;
      }
      setSavedContent(draft);
      setDigest(result.digest);
      setNotice({ kind: "success", text: "已保存" });
    } catch (reason) {
      setNotice({ kind: "error", text: describeSaveError(reason) });
    } finally {
      setSaving(false);
    }
  }, [projectId, lcId, selectedPath, draft, digest]);

  const tree = useMemo(() => buildTree(nodes), [nodes]);

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 px-4 py-3">
      <header className="flex flex-wrap items-center gap-3">
        <h1 className="text-base font-semibold text-[var(--aria-ink)]">知识库</h1>
        <span data-testid="kb-status" className="text-xs text-[var(--aria-ink-muted)]">
          {status === null
            ? "知识库状态未知"
            : status.exists
              ? "知识库已初始化"
              : "知识库尚未初始化"}
        </span>
        {dirty ? (
          <span data-testid="kb-dirty" className="text-xs text-amber-700">
            有未保存的修改
          </span>
        ) : null}
      </header>

      <div className="flex min-h-0 flex-1 gap-4">
        <aside className="w-72 shrink-0 overflow-auto rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2">
          <div data-testid="kb-file-tree">
            <TreeView
              entries={tree}
              selectedPath={selectedPath}
              onSelect={(path) => void openFile(path)}
            />
          </div>
        </aside>

        <section className="flex min-w-0 flex-1 flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <div role="tablist" className="flex items-center gap-1">
              <button
                type="button"
                role="tab"
                aria-selected={tab === "edit"}
                onClick={() => setTab("edit")}
                className="rounded-md border border-[var(--aria-line)] bg-white px-3 py-1 text-xs font-medium text-[var(--aria-ink)]"
              >
                编辑
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={tab === "preview"}
                onClick={() => setTab("preview")}
                className="rounded-md border border-[var(--aria-line)] bg-white px-3 py-1 text-xs font-medium text-[var(--aria-ink)]"
              >
                预览
              </button>
            </div>
            <button
              type="button"
              onClick={() => void save()}
              disabled={selectedPath === null || digest === null || saving}
              className="rounded-md border border-[var(--aria-line)] bg-white px-3 py-1 text-xs font-semibold text-[var(--aria-ink)] disabled:opacity-50"
            >
              保存
            </button>
            {selectedPath ? (
              <span className="truncate text-xs text-[var(--aria-ink-muted)]">
                {selectedPath}
              </span>
            ) : null}
          </div>

          {notice ? (
            <div
              role={notice.kind === "error" ? "alert" : "status"}
              data-testid="kb-notice"
              data-kind={notice.kind}
              className={
                notice.kind === "error"
                  ? "rounded-md border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700"
                  : "rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2 text-xs text-emerald-700"
              }
            >
              {notice.text}
            </div>
          ) : null}

          <div role="tabpanel" className="min-h-0 flex-1">
            {tab === "edit" ? (
              <MonacoEditor
                value={draft}
                language="markdown"
                height="480px"
                onChange={setDraft}
              />
            ) : (
              <div className="rounded-md border border-[var(--aria-line)] bg-white p-3">
                <MarkdownContent content={draft} />
              </div>
            )}
          </div>

          {lintFields.length > 0 ? (
            <div
              data-testid="kb-lint-fields"
              className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
            >
              <div className="px-1 pb-1 text-xs font-semibold text-[var(--aria-ink)]">
                变更包字段状态
              </div>
              <ul className="space-y-1">
                {lintFields.map((field) => (
                  <li
                    key={`${field.document}::${field.field}`}
                    className="flex items-center gap-2 px-1 text-xs text-[var(--aria-ink)]"
                  >
                    <span className="truncate">
                      <span className="text-[var(--aria-ink-muted)]">
                        {field.document}
                      </span>
                      <span aria-hidden="true"> · </span>
                      <span data-testid="kb-lint-field">{field.field}</span>
                    </span>
                    <span
                      data-filled={String(field.filled)}
                      className={
                        field.filled
                          ? "rounded border border-emerald-200 bg-emerald-50 px-1.5 py-0.5 text-[11px] text-emerald-700"
                          : "rounded border border-amber-200 bg-amber-50 px-1.5 py-0.5 text-[11px] text-amber-800"
                      }
                    >
                      {field.filled ? "已填" : "未填"}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </section>
      </div>
    </div>
  );
}

function TreeView({
  entries,
  selectedPath,
  onSelect,
  level = "tree",
}: {
  entries: TreeEntry[];
  selectedPath: string | null;
  onSelect: (path: string) => void;
  /** 根层为 tree，目录内的子层为 group（避免嵌套出多棵树）。 */
  level?: "tree" | "group";
}) {
  if (entries.length === 0) {
    return (
      <p className="px-1 py-2 text-xs text-[var(--aria-ink-muted)]">知识库暂无文档</p>
    );
  }
  return (
    <ul
      role={level}
      className={level === "tree" ? "space-y-1" : "ml-3 space-y-1"}
    >
      {entries.map((entry) =>
        entry.kind === "dir" ? (
          <li key={entry.path} role="treeitem" aria-expanded="true">
            <span
              data-testid="kb-dir-node"
              data-path={entry.path}
              className="block px-1 text-xs font-semibold text-[var(--aria-ink-muted)]"
            >
              {entry.name}
            </span>
            <TreeView
              level="group"
              entries={entry.children}
              selectedPath={selectedPath}
              onSelect={onSelect}
            />
          </li>
        ) : (
          <li key={entry.path} role="treeitem">
            <button
              type="button"
              data-testid="kb-file-node"
              data-path={entry.path}
              title={entry.path}
              aria-current={selectedPath === entry.path}
              onClick={() => onSelect(entry.path)}
              className={
                selectedPath === entry.path
                  ? "w-full truncate rounded px-2 py-1 text-left text-xs font-semibold text-[var(--aria-ink)] bg-white"
                  : "w-full truncate rounded px-2 py-1 text-left text-xs text-[var(--aria-ink)] hover:bg-white"
              }
            >
              {entry.name}
            </button>
          </li>
        ),
      )}
    </ul>
  );
}

function buildTree(nodes: KnowledgeBaseFileNodeDto[]): TreeEntry[] {
  const dirs = new Map<string, TreeEntry & { kind: "dir" }>();
  // 根容器：父路径为 null 的目录与文件（顶层目录必须挂在这里，否则整棵目录分组不可达）。
  const roots: TreeEntry[] = [];

  const ensureDir = (path: string): TreeEntry & { kind: "dir" } => {
    const existing = dirs.get(path);
    if (existing) {
      return existing;
    }
    const created: TreeEntry & { kind: "dir" } = {
      kind: "dir",
      name: path.split("/").filter(Boolean).at(-1) ?? path,
      path,
      children: [],
    };
    dirs.set(path, created);
    const parentPath = parentOf(path);
    if (parentPath === null) {
      roots.push(created);
    } else {
      ensureDir(parentPath).children.push(created);
    }
    return created;
  };

  for (const node of nodes) {
    if (node.is_dir) {
      ensureDir(node.path);
      continue;
    }
    const entry: TreeEntry & { kind: "file" } = {
      kind: "file",
      name: node.path.split("/").at(-1) ?? node.path,
      path: node.path,
    };
    const parentPath = parentOf(node.path);
    if (parentPath === null) {
      roots.push(entry);
    } else {
      ensureDir(parentPath).children.push(entry);
    }
  }

  return sortEntries(roots);
}

function sortEntries(entries: TreeEntry[]): TreeEntry[] {
  const sorted = [...entries].sort((left, right) => {
    if (left.kind !== right.kind) {
      return left.kind === "dir" ? -1 : 1;
    }
    return left.name.localeCompare(right.name);
  });
  return sorted.map((entry) =>
    entry.kind === "dir"
      ? {
          kind: "dir",
          name: entry.name,
          path: entry.path,
          children: sortEntries(entry.children),
        }
      : entry,
  );
}

function parentOf(path: string): string | null {
  const index = path.lastIndexOf("/");
  if (index < 0) {
    return null;
  }
  return path.slice(0, index);
}

/** 变更包目录（user-input/updates/CHANGE-*）下的文件归属的变更包路径。 */
function changePackagePathOf(path: string): string | null {
  const index = path.indexOf(CHANGE_PACKAGE_SEGMENT);
  if (index < 0) {
    return null;
  }
  const rest = path.slice(index + CHANGE_PACKAGE_SEGMENT.length);
  const [packageName, ...fileSegments] = rest.split("/");
  if (!packageName || fileSegments.length === 0) {
    return null;
  }
  return `${path.slice(0, index + CHANGE_PACKAGE_SEGMENT.length)}${packageName}`;
}

function errorText(reason: unknown): string {
  if (reason instanceof Error) {
    return reason.message;
  }
  return "未知错误";
}

/**
 * 保存失败的文案：区分「内容冲突（digest 不符）」与「知识库操作进行中」，
 * 两者都必须明确提示，既不静默失败也不自动重试覆盖。
 */
function describeSaveError(reason: unknown): string {
  if (reason instanceof ApiRequestError) {
    if (reason.code === "knowledge_base_operation_running") {
      return "知识库操作进行中，请等待当前操作结束后再保存。";
    }
    if (reason.code === "knowledge_base_file_conflict") {
      return "内容已被修改（与服务端版本冲突），请重新加载文件后再保存。";
    }
    if (/操作进行中|operation[_ -]?running/i.test(reason.message)) {
      return "知识库操作进行中，请等待当前操作结束后再保存。";
    }
    if (/已被修改|冲突|conflict/i.test(reason.message)) {
      return "内容已被修改（与服务端版本冲突），请重新加载文件后再保存。";
    }
    return `保存失败：${reason.message}`;
  }
  return `保存失败：${errorText(reason)}`;
}
