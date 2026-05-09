import type { ArtifactContentResponse } from "../../api/types";
import { ArtifactContentRenderer } from "./ArtifactContentRenderer";

export function ArtifactViewer({ artifact }: { artifact: ArtifactContentResponse | null }) {
  if (!artifact) {
    return <div className="text-sm text-slate-500">未选择 artifact。</div>;
  }

  return (
    <section className="rounded-md border border-line bg-white">
      <header className="border-b border-line px-3 py-2">
        <h3 className="text-sm font-semibold">{artifact.artifact_kind}</h3>
        <p className="truncate text-xs text-slate-500">
          {artifact.producer_node ?? "unknown node"} · {artifact.path}
        </p>
      </header>
      <ArtifactContentRenderer contentType={artifact.content_type} content={artifact.content} />
    </section>
  );
}
