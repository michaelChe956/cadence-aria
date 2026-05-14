export function EvidencePanel({
  artifacts,
  diagnostics,
}: {
  artifacts: Array<Record<string, unknown>>;
  diagnostics: Array<Record<string, unknown>>;
}) {
  return (
    <aside className="h-fit rounded-lg border-2 border-emerald-200 bg-white/90 p-3 shadow-[0_10px_0_rgba(16,185,129,0.10),0_18px_34px_rgba(16,185,129,0.14)] lg:sticky lg:top-28">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-sm font-black text-[#241B2F]">Evidence</h2>
        <span className="rounded-lg border-2 border-emerald-200 bg-emerald-50 px-2 py-1 font-mono text-[11px] font-bold text-emerald-700">
          {artifacts.length + diagnostics.length} items
        </span>
      </div>
      <section className="mt-3">
        <div className="flex items-center justify-between">
          <h3 className="text-[11px] font-black uppercase text-emerald-700">Reports</h3>
          <span className="font-mono text-[11px] font-bold text-emerald-700">
            {artifacts.length}
          </span>
        </div>
        {artifacts.length === 0 ? (
          <EmptyState>暂无产物</EmptyState>
        ) : (
          artifacts.map((artifact) => <ArtifactReportCard key={String(artifact.artifact_ref)} artifact={artifact} />)
        )}
      </section>
      <section className="mt-4">
        <div className="flex items-center justify-between">
          <h3 className="text-[11px] font-bold uppercase text-orange-700">
            Diagnostics
          </h3>
          <span className="font-mono text-[11px] font-bold text-orange-700">
            {diagnostics.length}
          </span>
        </div>
        {diagnostics.length === 0 ? (
          <EmptyState>暂无诊断</EmptyState>
        ) : (
          diagnostics.map((diagnostic, index) => (
            <div
              key={index}
              className="mt-2 rounded-lg border-2 border-orange-200 bg-orange-100 px-3 py-2 text-sm font-semibold text-orange-900"
            >
              {String(diagnostic.message ?? diagnostic.code)}
            </div>
          ))
        )}
      </section>
    </aside>
  );
}

function ArtifactReportCard({ artifact }: { artifact: Record<string, unknown> }) {
  return (
    <button
      type="button"
      className="mt-2 grid w-full grid-cols-[4rem_minmax(0,1fr)] gap-3 rounded-lg border-2 border-emerald-100 bg-gradient-to-br from-emerald-50 to-cyan-50 p-3 text-left text-sm font-semibold text-emerald-950 shadow-[0_5px_0_rgba(16,185,129,0.14)] transition-colors hover:border-orange-300 hover:bg-orange-50 focus-visible:border-orange-400 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200"
    >
      <ReportPreview />
      <span className="min-w-0">
        <span className="block text-sm font-black text-[#241B2F]">Artifact</span>
        <span className="mt-1 block rounded-md bg-white/80 px-2 py-1 font-mono text-xs text-emerald-700">
          {String(artifact.path)}
        </span>
        <span className="mt-2 inline-flex rounded-md border-2 border-emerald-200 bg-white px-2 py-0.5 text-[10px] font-black uppercase text-emerald-700">
          {String(artifact.artifact_kind)}
        </span>
      </span>
    </button>
  );
}

function ReportPreview() {
  return (
    <svg
      role="img"
      aria-label="artifact preview"
      viewBox="0 0 64 64"
      className="h-16 w-16 rounded-lg border-2 border-white bg-white shadow-[0_5px_0_rgba(16,185,129,0.16)]"
    >
      <title>artifact preview</title>
      <rect x="12" y="8" width="40" height="48" rx="8" fill="#FFF4EC" />
      <rect x="18" y="16" width="20" height="6" rx="3" fill="#8E2D60" />
      <rect x="18" y="28" width="28" height="5" rx="2.5" fill="#0F766E" />
      <rect x="18" y="39" width="18" height="5" rx="2.5" fill="#F97316" />
      <circle cx="44" cy="44" r="8" fill="#DCFCE7" />
      <path d="m40 44 3 3 6-7" stroke="#10B981" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function EmptyState({ children }: { children: string }) {
  return (
    <div className="mt-2 rounded-lg border-2 border-dashed border-emerald-200 bg-emerald-50/70 px-3 py-3 text-sm font-semibold text-emerald-700">
      {children}
    </div>
  );
}
