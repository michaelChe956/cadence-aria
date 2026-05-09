export function EvidencePanel({
  artifacts,
  diagnostics,
}: {
  artifacts: Array<Record<string, unknown>>;
  diagnostics: Array<Record<string, unknown>>;
}) {
  return (
    <aside className="border-l border-line bg-white p-3">
      <h2 className="text-sm font-semibold">Evidence</h2>
      <section className="mt-3">
        <h3 className="text-xs font-semibold uppercase text-slate-500">Artifacts</h3>
        {artifacts.map((artifact) => (
          <button
            key={String(artifact.artifact_ref)}
            type="button"
            className="mt-2 block w-full rounded-md border border-line px-2 py-2 text-left text-sm"
          >
            <span className="font-medium">{String(artifact.artifact_kind)}</span>
            <span className="block truncate text-xs text-slate-500">{String(artifact.path)}</span>
          </button>
        ))}
      </section>
      <section className="mt-4">
        <h3 className="text-xs font-semibold uppercase text-slate-500">Diagnostics</h3>
        {diagnostics.map((diagnostic, index) => (
          <div key={index} className="mt-2 rounded-md border border-caution/40 bg-amber-50 px-2 py-2 text-sm">
            {String(diagnostic.message ?? diagnostic.code)}
          </div>
        ))}
      </section>
    </aside>
  );
}
