export function EvidencePanel({
  artifacts,
  diagnostics,
}: {
  artifacts: Array<Record<string, unknown>>;
  diagnostics: Array<Record<string, unknown>>;
}) {
  return (
    <aside className="rounded-xl border border-white/10 bg-white/[0.03] p-3">
      <h2 className="text-sm font-semibold text-slate-100">Evidence</h2>
      <section className="mt-3">
        <h3 className="text-xs font-semibold uppercase text-slate-500">Artifacts</h3>
        {artifacts.map((artifact) => (
          <button
            key={String(artifact.artifact_ref)}
            type="button"
            className="mt-2 block w-full rounded-md border border-white/10 bg-black/25 px-2 py-2 text-left text-sm text-slate-200 hover:border-cyan-300/40"
          >
            <span className="font-medium">{String(artifact.artifact_kind)}</span>
            <span className="block truncate text-xs text-slate-500">{String(artifact.path)}</span>
          </button>
        ))}
      </section>
      <section className="mt-4">
        <h3 className="text-xs font-semibold uppercase text-slate-500">Diagnostics</h3>
        {diagnostics.map((diagnostic, index) => (
          <div key={index} className="mt-2 rounded-md border border-amber-300/40 bg-amber-300/10 px-2 py-2 text-sm text-amber-100">
            {String(diagnostic.message ?? diagnostic.code)}
          </div>
        ))}
      </section>
    </aside>
  );
}
