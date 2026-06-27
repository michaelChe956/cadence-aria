import { useEffect, useState } from "react";
import { getCodingAttemptDiff } from "../api/client";
import { MonacoDiffViewer } from "../components/shared/MonacoDiffViewer";
import { MonacoViewer } from "../components/shared/MonacoViewer";
import {
  type CodingArtifactTab,
  useCodingWorkspaceStore,
} from "../state/coding-workspace-store";
import { errorMessage } from "./CodingWorkspaceControls";
import { GitPanel, LogsPanel, ReviewPanel, TestsPanel } from "./CodingWorkspaceReports";

type CodingDiffState = {
  attemptId: string | null;
  status: "idle" | "loading" | "loaded" | "error";
  diff: string;
  error: string | null;
};


export function CodingArtifactTabs({
  activeTab,
  className = "",
}: {
  activeTab: CodingArtifactTab;
  className?: string;
}) {
  const attemptId = useCodingWorkspaceStore((state) => state.attemptId);
  const [diffState, setDiffState] = useState<CodingDiffState>({
    attemptId: null,
    status: "idle",
    diff: "",
    error: null,
  });
  const tabs: CodingArtifactTab[] = ["diff", "tests", "review", "git", "logs"];

  useEffect(() => {
    if (activeTab !== "diff" || !attemptId) {
      return;
    }
    if (diffState.attemptId === attemptId && diffState.status === "loaded") {
      return;
    }

    let cancelled = false;
    setDiffState({
      attemptId,
      status: "loading",
      diff: "",
      error: null,
    });
    getCodingAttemptDiff(attemptId)
      .then((response) => {
        if (cancelled) return;
        setDiffState({
          attemptId,
          status: "loaded",
          diff: response.diff,
          error: null,
        });
      })
      .catch((reason) => {
        if (cancelled) return;
        setDiffState({
          attemptId,
          status: "error",
          diff: "",
          error: errorMessage(reason, "加载代码变更失败"),
        });
      });

    return () => {
      cancelled = true;
    };
  }, [activeTab, attemptId]);

  return (
    <aside
      data-testid="coding-artifact-tabs"
      className={`flex min-h-0 flex-col bg-[var(--aria-panel)] ${className}`}
    >
      <div className="flex shrink-0 border-b border-[var(--aria-line)] px-2 py-2">
        {tabs.map((tab) => (
          <button
            key={tab}
            type="button"
            onClick={() => useCodingWorkspaceStore.getState().setActiveTab(tab)}
            className={[
              "h-8 rounded-md px-2 text-xs font-semibold",
              activeTab === tab
                ? "bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
                : "text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]",
            ].join(" ")}
          >
            {tab}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-3 text-sm">
        {activeTab === "tests" ? (
          <TestsPanel />
        ) : activeTab === "review" ? (
          <ReviewPanel />
        ) : activeTab === "git" ? (
          <GitPanel />
        ) : activeTab === "logs" ? (
          <LogsPanel />
        ) : (
          <DiffPanel diffState={diffState} />
        )}
      </div>
    </aside>
  );
}

function DiffPanel({ diffState }: { diffState: CodingDiffState }) {
  if (diffState.status === "loading") {
    return <div className="text-[var(--aria-ink-muted)]">正在加载代码变更...</div>;
  }
  if (diffState.status === "error") {
    return <div className="text-[var(--aria-danger)]">{diffState.error}</div>;
  }
  if (!diffState.diff.trim()) {
    return <div className="text-[var(--aria-ink-muted)]">暂无代码变更</div>;
  }
  const files = parseUnifiedDiff(diffState.diff);
  if (files.length > 0) {
    return (
      <div className="space-y-3">
        {files.map((file) => (
          <div
            key={file.id}
            data-testid="coding-diff-file"
            className="overflow-hidden rounded-md border border-[var(--aria-line)] bg-white"
          >
            <div className="flex h-9 min-w-0 items-center gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3">
              <span className="shrink-0 rounded bg-[var(--aria-panel-subtle)] px-1.5 py-0.5 text-[10px] font-semibold uppercase text-[var(--aria-ink-muted)]">
                {diffStatusLabel(file.status)}
              </span>
              <span className="truncate font-mono text-xs text-[var(--aria-ink)]">
                {file.path}
              </span>
            </div>
            {file.binary ? (
              <div className="px-3 py-2 text-xs text-[var(--aria-ink-muted)]">
                二进制文件变更未展示内容
              </div>
            ) : (
              <MonacoDiffViewer
                original={file.original}
                modified={file.modified}
                language={languageForPath(file.path)}
                height="min(62vh, 620px)"
              />
            )}
          </div>
        ))}
      </div>
    );
  }
  return (
    <div className="min-h-[420px] overflow-hidden rounded-md border border-[var(--aria-line)]">
      <MonacoViewer value={diffState.diff} language="diff" height="min(70vh, 720px)" />
    </div>
  );
}

type ParsedDiffFile = {
  id: string;
  path: string;
  oldPath: string;
  newPath: string;
  original: string;
  modified: string;
  status: "added" | "deleted" | "modified" | "renamed" | "binary";
  binary: boolean;
};

type MutableParsedDiffFile = Omit<ParsedDiffFile, "original" | "modified"> & {
  originalLines: string[];
  modifiedLines: string[];
  inHunk: boolean;
  hunkCount: number;
};

function parseUnifiedDiff(diff: string): ParsedDiffFile[] {
  const files: MutableParsedDiffFile[] = [];
  let current: MutableParsedDiffFile | null = null;
  const lines = diff.replace(/\r\n?/g, "\n").split("\n");

  function pushCurrent() {
    if (current) {
      files.push(current);
      current = null;
    }
  }

  for (const line of lines) {
    const header = parseDiffHeader(line);
    if (header) {
      pushCurrent();
      current = {
        id: `${files.length}:${header.oldPath}:${header.newPath}`,
        oldPath: header.oldPath,
        newPath: header.newPath,
        path: header.newPath !== "/dev/null" ? header.newPath : header.oldPath,
        status: "modified",
        binary: false,
        originalLines: [],
        modifiedLines: [],
        inHunk: false,
        hunkCount: 0,
      };
      continue;
    }
    if (!current) {
      continue;
    }
    if (line.startsWith("new file mode")) {
      current.status = "added";
      continue;
    }
    if (line.startsWith("deleted file mode")) {
      current.status = "deleted";
      continue;
    }
    if (line.startsWith("rename to ")) {
      current.status = "renamed";
      current.newPath = line.slice("rename to ".length).trim();
      current.path = current.newPath;
      continue;
    }
    if (line.startsWith("Binary files ")) {
      current.status = "binary";
      current.binary = true;
      continue;
    }
    if (line.startsWith("@@")) {
      if (current.hunkCount > 0) {
        current.originalLines.push("");
        current.modifiedLines.push("");
      }
      current.inHunk = true;
      current.hunkCount += 1;
      continue;
    }
    if (!current.inHunk || line.startsWith("\\ No newline")) {
      continue;
    }
    const marker = line[0];
    const text = line.slice(1);
    if (marker === " ") {
      current.originalLines.push(text);
      current.modifiedLines.push(text);
    } else if (marker === "-") {
      current.originalLines.push(text);
    } else if (marker === "+") {
      current.modifiedLines.push(text);
    }
  }
  pushCurrent();

  return files.map(({ originalLines, modifiedLines, inHunk, hunkCount, ...file }) => ({
    ...file,
    original: originalLines.join("\n"),
    modified: modifiedLines.join("\n"),
  }));
}

function parseDiffHeader(line: string) {
  const match = /^diff --git a\/(.+) b\/(.+)$/.exec(line);
  if (!match) {
    return null;
  }
  return {
    oldPath: match[1],
    newPath: match[2],
  };
}

function diffStatusLabel(status: ParsedDiffFile["status"]) {
  switch (status) {
    case "added":
      return "新增";
    case "deleted":
      return "删除";
    case "renamed":
      return "重命名";
    case "binary":
      return "二进制";
    default:
      return "修改";
  }
}

function languageForPath(path: string) {
  const extension = path.split(".").pop()?.toLowerCase();
  switch (extension) {
    case "py":
      return "python";
    case "ts":
    case "tsx":
      return "typescript";
    case "js":
    case "jsx":
      return "javascript";
    case "json":
      return "json";
    case "md":
      return "markdown";
    case "rs":
      return "rust";
    case "toml":
      return "toml";
    case "yml":
    case "yaml":
      return "yaml";
    case "sh":
      return "shell";
    case "css":
      return "css";
    case "html":
      return "html";
    default:
      return "plaintext";
  }
}

