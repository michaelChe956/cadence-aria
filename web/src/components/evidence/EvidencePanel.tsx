export function EvidencePanel({
  artifacts,
  diagnostics,
}: {
  artifacts: Array<Record<string, unknown>>;
  diagnostics: Array<Record<string, unknown>>;
}) {
  return (
    <aside className="h-fit rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 lg:sticky lg:top-28">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">Evidence</h2>
        <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 font-mono text-[11px] font-semibold text-[var(--aria-ink-muted)]">
          {artifacts.length + diagnostics.length} items
        </span>
      </div>
      <section className="mt-3">
        <div className="flex items-center justify-between">
          <h3 className="text-[11px] font-semibold uppercase text-[var(--aria-ink-muted)]">
            Reports
          </h3>
          <span className="font-mono text-[11px] font-semibold text-[var(--aria-ink-muted)]">
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
          <h3 className="text-[11px] font-semibold uppercase text-[var(--aria-ink-muted)]">
            Diagnostics
          </h3>
          <span className="font-mono text-[11px] font-semibold text-[var(--aria-ink-muted)]">
            {diagnostics.length}
          </span>
        </div>
        {diagnostics.length === 0 ? (
          <EmptyState>暂无诊断</EmptyState>
        ) : (
          diagnostics.map((diagnostic, index) => (
            <div
              key={index}
              className="mt-2 rounded-md border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-sm font-medium text-[var(--aria-ink)]"
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
      className="mt-2 grid w-full grid-cols-[4rem_minmax(0,1fr)] gap-3 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3 text-left text-sm font-medium text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
    >
      <ReportPreview />
      <span className="min-w-0">
        <span className="block text-sm font-semibold text-[var(--aria-ink)]">Artifact</span>
        <span className="mt-1 block rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-1 font-mono text-xs text-[var(--aria-ink-muted)]">
          {String(artifact.path)}
        </span>
        <span className="mt-2 inline-flex rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 text-[10px] font-semibold uppercase text-[var(--aria-ink-muted)]">
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
      className="h-16 w-16 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)]"
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
    <div className="mt-2 rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-3 text-sm font-medium text-[var(--aria-ink-muted)]">
      {children}
    </div>
  );
}
