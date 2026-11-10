// Task 10：LC 知识库详情页。
// 全屏工作区形态（Task 7）：顶栏（标题 + 所属项目名/逻辑代码库名 + 状态徽标 +
// 返回工作台）+ 主体两栏——左侧文件树（固定宽、可折叠）+ 右侧「编辑 / 预览」
// 自适应并吃满可用高度（滚动只发生在文件树与编辑区内部，下半屏不留空白）。
// 编辑页签用 Task 8 的可写 MonacoEditor，预览页签用公共 MarkdownContent；
// 变更包（user-input/updates/ 下的 CHANGE-* 目录）额外展示 lint 的字段级标记。
// 机器管理文件（后端标记 editable=false 的 manifest.yaml、
// evidence/relation-graph.yaml 等）按只读呈现：不提供编辑入口与保存入口。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ApiRequestError, listProjects } from "../api/client";
import { listLogicalCodebases } from "../api/codebases";
import {
  getKnowledgeBaseStatus,
  getKnowledgeBaseUserInput,
  lintKnowledgeBaseChangePackage,
  listKnowledgeBaseFiles,
  readKnowledgeBaseFile,
  writeKnowledgeBaseFile,
} from "../api/knowledge-base";
import type {
  KnowledgeBaseChangePackageFieldStatusDto,
  KnowledgeBaseFileNodeDto,
  KnowledgeBaseStatusDto,
  KnowledgeBaseUserInputSummaryDto,
} from "../api/types/knowledge-base";
import { KnowledgeBaseUserInputPanel } from "../components/lifecycle/KnowledgeBaseUserInputPanel";
import { MarkdownContent } from "../components/shared/MarkdownContent";
import { MonacoEditor } from "../components/shared/MonacoEditor";
import { useUnloadGuard } from "../hooks/useUnloadGuard";

/** 变更包根段：文件树 path 是知识库根相对口径（user-input/updates/CHANGE-*）；
 *  匹配同时兼容历史聚合根相对口径（…/user-input/updates/CHANGE-*）。 */
const CHANGE_PACKAGE_SEGMENT = "user-input/updates/";
const UNSAVED_MESSAGE = "知识库文档有未保存的修改，离开将丢失这些改动。是否继续？";
/**
 * REQ-KB-13 前置门：聚合初始化未完成时，知识库端点（含状态端点）在 HTTP 层
 * 以该稳定码拒绝。前置未满足的判定只依据后端已给出的稳定码——不臆造
 * status 投影里尚不存在的字段（status 缺该信号时不显示「前置未满足」徽标）。
 */
const PREFLIGHT_REQUIRED_CODE = "knowledge_base_requires_aggregate_initialization";

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
  // 顶栏副标题的关联名：项目名 + 逻辑代码库名（取不到时回落 id，不留空）。
  const [projectName, setProjectName] = useState<string | null>(null);
  const [logicalCodebaseName, setLogicalCodebaseName] = useState<string | null>(null);
  // 前置门未满足（后端以稳定码拒绝）⇒ 顶栏显示「前置未满足」徽标。
  const [preflightUnmet, setPreflightUnmet] = useState(false);
  // 左侧文件树可折叠（固定宽，折叠后右区获得全部宽度）。
  const [treeCollapsed, setTreeCollapsed] = useState(false);
  // 当前选中的文件路径（同步镜像）：用于丢弃乱序到达的过期读取响应。
  const selectedPathRef = useRef<string | null>(null);
  // 输入确认区（REQ-KB-15）：9 份输入的状态投影；不可用（前置门未满足 /
  // 后端未就绪）时为 null ⇒ 确认区整体隐藏，不阻塞文件浏览。
  const [userInput, setUserInput] = useState<KnowledgeBaseUserInputSummaryDto | null>(
    null,
  );
  // 已确认后被编辑（摘要漂移）的输入文件：逐份提示「已修改，需重新确认」。
  const [userInputInvalidated, setUserInputInvalidated] = useState<
    ReadonlySet<string>
  >(new Set());
  const confirmedPathsRef = useRef<ReadonlySet<string>>(new Set());
  const userInputPathsRef = useRef<ReadonlySet<string>>(new Set());
  const userInputEpochRef = useRef(0);

  // 选中文件的节点元数据：机器管理文件按只读呈现——编辑器不传 onChange
  // （MonacoEditor 的「无 onChange ⇒ 只读」内建语义）、保存入口禁用、并明确
  // 标注只读说明；可编辑文件维持「可写 + 可保存」现状。
  const selectedNode =
    selectedPath === null
      ? null
      : (nodes.find((node) => node.path === selectedPath) ?? null);
  const selectedEditable = selectedNode?.editable ?? false;

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
          // 同路由仅换 LC 参数重跑副作用时，成功路径须清零「前置未满足」，
          // 否则旧徽标会残留。
          setPreflightUnmet(false);
        }
      })
      .catch((reason) => {
        // status 不可用不阻塞文件浏览（后端未就绪时的正常降级）。
        if (cancelled) {
          return;
        }
        setStatus(null);
        // 前置门拒绝（聚合初始化未完成）⇒ 顶栏显示「前置未满足」。
        setPreflightUnmet(
          reason instanceof ApiRequestError && reason.code === PREFLIGHT_REQUIRED_CODE,
        );
      });
    return () => {
      cancelled = true;
    };
  }, [projectId, lcId, openFile]);

  // 顶栏关联名：项目名 + 逻辑代码库名（既有只读端点；取不到时回落 id）。
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const [projectsResult, logicalCodebasesResult] = await Promise.allSettled([
        listProjects(),
        listLogicalCodebases(projectId),
      ]);
      if (cancelled) {
        return;
      }
      if (projectsResult.status === "fulfilled") {
        const matched = projectsResult.value.projects.find(
          (item) => item.project_id === projectId,
        );
        if (matched) {
          setProjectName(matched.name);
        }
      }
      if (logicalCodebasesResult.status === "fulfilled") {
        const matched = logicalCodebasesResult.value.logical_codebases.find(
          (item) => item.id === lcId,
        );
        if (matched) {
          setLogicalCodebaseName(matched.name);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [projectId, lcId]);

  /** 应用新的输入投影：记录「已确认 → 待确认」迁移（编辑使确认按 digest 失效）。 */
  const applyUserInputSummary = useCallback(
    (next: KnowledgeBaseUserInputSummaryDto) => {
      const confirmed = new Set(
        next.files.filter((file) => file.confirmed).map((file) => file.path),
      );
      const pending = new Set(
        next.files.filter((file) => file.pending).map((file) => file.path),
      );
      // 迁移必须在 setState 外用「更新前」的 ref 基线急切计算：React 的
      // state updater 在下次渲染才执行，届时 ref 已指向新基线会漏判。
      const lapsed = [...confirmedPathsRef.current].filter((path) =>
        pending.has(path),
      );
      setUserInputInvalidated((prev) => {
        const merged = new Set(prev);
        for (const path of lapsed) {
          merged.add(path);
        }
        for (const path of confirmed) {
          merged.delete(path);
        }
        return merged;
      });
      confirmedPathsRef.current = confirmed;
      userInputPathsRef.current = new Set(next.files.map((file) => file.path));
      setUserInput(next);
    },
    [],
  );

  /** 拉取输入确认投影；换 LC 的乱序响应按轮次丢弃。失败 ⇒ 静默隐藏确认区。 */
  const refreshUserInput = useCallback(async () => {
    const epoch = ++userInputEpochRef.current;
    try {
      const next = await getKnowledgeBaseUserInput(projectId, lcId);
      if (epoch !== userInputEpochRef.current) {
        return;
      }
      applyUserInputSummary(next);
    } catch {
      if (epoch !== userInputEpochRef.current) {
        return;
      }
      setUserInput(null);
    }
  }, [projectId, lcId, applyUserInputSummary]);

  // 输入确认区随路由参数加载；换 LC 时先清零失效集合，避免旧 LC 的
  // 「已确认」基线把新 LC 的文件误标为需重新确认。
  useEffect(() => {
    confirmedPathsRef.current = new Set();
    setUserInputInvalidated(new Set());
    void refreshUserInput();
  }, [refreshUserInput]);

  const save = useCallback(async () => {
    if (!selectedPath || !selectedEditable) {
      // 未选中文件或机器管理文件（只读）：不发写请求（按钮已禁用，这里兜底）。
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
      // 保存的是输入文件 ⇒ 刷新输入投影（编辑会使该份确认按 digest 自动失效）。
      if (userInputPathsRef.current.has(selectedPath)) {
        void refreshUserInput();
      }
    } catch (reason) {
      setNotice({ kind: "error", text: describeSaveError(reason) });
    } finally {
      setSaving(false);
    }
  }, [
    projectId,
    lcId,
    selectedPath,
    selectedEditable,
    draft,
    digest,
    refreshUserInput,
  ]);

  const tree = useMemo(() => buildTree(nodes), [nodes]);

  // 关联名回落：取不到时用 id（副标题不留空）。
  const projectLabel = projectName ?? projectId;
  const logicalCodebaseLabel = logicalCodebaseName ?? lcId;

  return (
    <div
      data-testid="kb-workspace"
      className="flex h-full min-h-0 flex-col bg-[var(--aria-bg)]"
    >
      {/* 顶栏：标题 + 关联名 + 状态徽标 + 返回工作台（页头固定，不随内容滚动）。 */}
      <header
        data-testid="kb-header"
        className="flex shrink-0 items-center gap-3 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2"
      >
        <div className="min-w-0">
          <h1 className="text-base font-semibold text-[var(--aria-ink)]">知识库</h1>
          <p
            data-testid="kb-context"
            className="truncate text-xs text-[var(--aria-ink-muted)]"
          >
            {projectLabel} · {logicalCodebaseLabel}
          </p>
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-2">
          {preflightUnmet ? (
            <span
              data-testid="kb-status"
              data-status="preflight-unmet"
              className="rounded-full border border-amber-300 bg-amber-50 px-2 py-0.5 text-xs font-medium text-amber-800"
            >
              前置未满足
            </span>
          ) : (
            <span
              data-testid="kb-status"
              data-status={
                status === null
                  ? "unknown"
                  : status.exists
                    ? "initialized"
                    : "not-initialized"
              }
              className="rounded-full border border-[var(--aria-line)] bg-white px-2 py-0.5 text-xs font-medium text-[var(--aria-ink-muted)]"
            >
              {status === null
                ? "状态未知"
                : status.exists
                  ? "已初始化"
                  : "尚未初始化"}
            </span>
          )}
          {dirty ? (
            <span
              data-testid="kb-dirty"
              className="rounded-full border border-amber-200 bg-amber-50 px-2 py-0.5 text-xs font-medium text-amber-800"
            >
              有未保存的修改
            </span>
          ) : null}
          <a
            data-testid="kb-back-to-workbench"
            href="/workbench"
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)]"
          >
            返回工作台
          </a>
        </div>
      </header>

      {/* 主体：左文件树（固定宽、可折叠）+ 右编辑/预览（自适应并吃满高度）。 */}
      <div className="flex min-h-0 flex-1">
        <aside
          data-testid="kb-file-tree-panel"
          className={`flex shrink-0 flex-col gap-2 overflow-auto border-r border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 ${
            treeCollapsed ? "w-14" : "w-72"
          }`}
        >
          <button
            type="button"
            data-testid="kb-tree-toggle"
            aria-expanded={!treeCollapsed}
            onClick={() => setTreeCollapsed((collapsed) => !collapsed)}
            className="whitespace-nowrap rounded-md border border-[var(--aria-line)] bg-white px-1.5 py-1 text-xs font-medium text-[var(--aria-ink)] transition-colors duration-200 hover:bg-[var(--aria-panel)]"
          >
            {treeCollapsed ? "展开" : "折叠"}
          </button>
          {treeCollapsed ? null : (
            <>
              <div data-testid="kb-file-tree">
                <TreeView
                  entries={tree}
                  selectedPath={selectedPath}
                  onSelect={(path) => void openFile(path)}
                />
              </div>
              {userInput ? (
                <KnowledgeBaseUserInputPanel
                  projectId={projectId}
                  lcId={lcId}
                  summary={userInput}
                  invalidatedPaths={userInputInvalidated}
                  selectedPath={selectedPath}
                  onOpenFile={(path) => void openFile(path)}
                  onSummaryChanged={() => void refreshUserInput()}
                />
              ) : null}
            </>
          )}
        </aside>

        <section
          data-testid="kb-editor-panel"
          className="flex min-h-0 min-w-0 flex-1 flex-col gap-2 p-4"
        >
          <div className="flex shrink-0 flex-wrap items-center gap-2">
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
              disabled={
                selectedPath === null ||
                digest === null ||
                saving ||
                !selectedEditable
              }
              className="rounded-md border border-[var(--aria-line)] bg-white px-3 py-1 text-xs font-semibold text-[var(--aria-ink)] disabled:opacity-50"
            >
              保存
            </button>
            {selectedPath !== null && !selectedEditable ? (
              <span
                data-testid="kb-readonly-hint"
                className="text-xs text-[var(--aria-ink-muted)]"
              >
                机器管理文件，只读
              </span>
            ) : null}
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
                  ? "shrink-0 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700"
                  : "shrink-0 rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2 text-xs text-emerald-700"
              }
            >
              {notice.text}
            </div>
          ) : null}

          {/* 编辑/预览区：吃掉右栏全部剩余高度（滚动只发生在本区内部）。 */}
          <div
            role="tabpanel"
            data-testid="kb-tabpanel"
            className="min-h-0 flex-1 overflow-hidden"
          >
            {tab === "edit" ? (
              <div
                data-testid="kb-editor-host"
                className="h-full min-h-0 overflow-hidden rounded-md border border-[var(--aria-line)] bg-white"
              >
                {/* 折叠文件树会改变右栏宽度；@monaco-editor/react 默认开启
                    automaticLayout，Monaco 会自行重排，故此处不做 key 重挂载
                    （避免清空撤销栈、重置光标与选区、打断输入法）。 */}
                <MonacoEditor
                  value={draft}
                  language="markdown"
                  height="100%"
                  onChange={selectedEditable ? setDraft : undefined}
                />
              </div>
            ) : (
              <div
                data-testid="kb-preview"
                className="h-full min-h-0 overflow-auto rounded-md border border-[var(--aria-line)] bg-white p-3"
              >
                <MarkdownContent content={draft} />
              </div>
            )}
          </div>

          {lintFields.length > 0 ? (
            <div
              data-testid="kb-lint-fields"
              className="max-h-40 shrink-0 overflow-auto rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
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
  // index=0 是知识库根相对的前缀形态；index>0 必须恰好落在目录边界
  // （前一字符是 /），排除子串撞名（如 axuser-input/updates/…）。
  if (index < 0 || (index > 0 && path[index - 1] !== "/")) {
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
