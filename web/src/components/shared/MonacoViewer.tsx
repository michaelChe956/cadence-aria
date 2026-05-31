import { lazy, Suspense } from "react";
import { useSystemMonacoTheme } from "./monaco-theme";

const Editor = lazy(() =>
  import("@monaco-editor/react").then((module) => ({ default: module.Editor })),
);

interface MonacoViewerProps {
  value: string;
  language?: string;
  height?: string;
}

export function MonacoViewer({
  value,
  language = "markdown",
  height = "300px",
}: MonacoViewerProps) {
  const theme = useSystemMonacoTheme();

  return (
    <Suspense fallback={<ViewerSkeleton height={height} />}>
      <Editor
        height={height}
        language={language}
        value={value}
        loading={<ViewerSkeleton height={height} />}
        options={{
          readOnly: true,
          minimap: { enabled: false },
          wordWrap: "on",
          lineNumbers: "on",
          scrollBeyondLastLine: false,
          folding: true,
          renderLineHighlight: "none",
          overviewRulerLanes: 0,
          hideCursorInOverviewRuler: true,
          contextmenu: false,
          domReadOnly: true,
        }}
        theme={theme}
      />
    </Suspense>
  );
}

function ViewerSkeleton({ height }: { height: string }) {
  return (
    <div
      data-testid="monaco-viewer-skeleton"
      style={{ height }}
      className="animate-pulse rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
    />
  );
}
