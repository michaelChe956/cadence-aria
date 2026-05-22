import { lazy, Suspense } from "react";
import { useSystemMonacoTheme } from "./monaco-theme";

const DiffEditor = lazy(() =>
  import("@monaco-editor/react").then((module) => ({ default: module.DiffEditor })),
);

interface MonacoDiffViewerProps {
  original: string;
  modified: string;
  language?: string;
  height?: string;
  sideBySide?: boolean;
}

export function MonacoDiffViewer({
  original,
  modified,
  language = "markdown",
  height = "400px",
  sideBySide = true,
}: MonacoDiffViewerProps) {
  const theme = useSystemMonacoTheme();

  return (
    <Suspense fallback={<DiffSkeleton height={height} />}>
      <DiffEditor
        height={height}
        language={language}
        original={original}
        modified={modified}
        loading={<DiffSkeleton height={height} />}
        keepCurrentOriginalModel
        keepCurrentModifiedModel
        options={{
          readOnly: true,
          renderSideBySide: sideBySide,
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          renderOverviewRuler: false,
          contextmenu: false,
          domReadOnly: true,
        }}
        theme={theme}
      />
    </Suspense>
  );
}

function DiffSkeleton({ height }: { height: string }) {
  return (
    <div
      data-testid="monaco-diff-skeleton"
      style={{ height }}
      className="animate-pulse rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
    />
  );
}
