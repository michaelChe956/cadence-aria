import type { ArtifactContentResponse } from "../../api/types";
import { ArtifactContentRenderer } from "./ArtifactContentRenderer";

export function ArtifactViewer({ artifact }: { artifact: ArtifactContentResponse | null }) {
  if (!artifact) {
    return <div className="text-sm font-semibold text-indigo-600">未选择 artifact。</div>;
  }

  return (
    <section className="rounded-lg border-2 border-indigo-200 bg-white text-indigo-950 shadow-[0_8px_0_rgba(129,140,248,0.12)]">
      <header className="border-b-2 border-indigo-100 px-3 py-2">
        <h3 className="text-sm font-bold text-indigo-950">{artifact.artifact_kind}</h3>
        <p className="truncate text-xs font-semibold text-indigo-600">
          {artifact.producer_node ?? "unknown node"} · {artifact.path}
        </p>
      </header>
      <ArtifactContentRenderer contentType={artifact.content_type} content={artifact.content} />
    </section>
  );
}
