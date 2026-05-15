import type { ArtifactContentResponse } from "../../api/types";
import { ArtifactContentRenderer } from "./ArtifactContentRenderer";

export function ArtifactViewer({ artifact }: { artifact: ArtifactContentResponse | null }) {
  if (!artifact) {
    return <div className="text-sm font-medium text-[var(--aria-ink-muted)]">未选择 artifact。</div>;
  }

  return (
    <section className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink)]">
      <header className="border-b border-[var(--aria-line)] px-3 py-2">
        <h3 className="text-sm font-semibold text-[var(--aria-ink)]">{artifact.artifact_kind}</h3>
        <p className="truncate font-mono text-xs font-medium text-[var(--aria-ink-muted)]">
          {artifact.producer_node ?? "unknown node"} · {artifact.path}
        </p>
      </header>
      <ArtifactContentRenderer contentType={artifact.content_type} content={artifact.content} />
    </section>
  );
}
