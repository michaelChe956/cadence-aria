import { lazy, Suspense } from "react";
// ★必须先于 @monaco-editor/react 初始化 loader（见 monaco-local-setup.ts 的说明）★
import "./monaco-local-setup";
import { useSystemMonacoTheme } from "./monaco-theme";

const Editor = lazy(() =>
  import("@monaco-editor/react").then((module) => ({ default: module.Editor })),
);

interface MonacoEditorProps {
  value: string;
  language?: string;
  height?: string;
  /** 传入则编辑器可写并把输入回传；不传则渲染为只读（readOnly 语义同 MonacoViewer）。 */
  onChange?: (value: string) => void;
}

export function MonacoEditor({
  value,
  language = "markdown",
  height = "400px",
  onChange,
}: MonacoEditorProps) {
  const theme = useSystemMonacoTheme();

  return (
    <Suspense fallback={<EditorSkeleton height={height} />}>
      <Editor
        height={height}
        language={language}
        value={value}
        loading={<EditorSkeleton height={height} />}
        onChange={(next) => onChange?.(next ?? "")}
        options={{
          readOnly: !onChange,
          minimap: { enabled: false },
          wordWrap: "on",
          lineNumbers: "on",
          scrollBeyondLastLine: false,
          folding: true,
          renderLineHighlight: "none",
          overviewRulerLanes: 0,
          hideCursorInOverviewRuler: true,
          contextmenu: false,
        }}
        theme={theme}
      />
    </Suspense>
  );
}

function EditorSkeleton({ height }: { height: string }) {
  return (
    <div
      data-testid="monaco-editor-skeleton"
      style={{ height }}
      className="animate-pulse rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
    >
      <span className="px-3 py-2 text-sm text-[var(--aria-ink-muted)]">正在加载内容…</span>
    </div>
  );
}
