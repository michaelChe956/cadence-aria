export function ImageCreatePage({ sessionId }: { sessionId?: string }) {
  return (
    <main
      aria-label="图片创作"
      className="min-h-screen bg-[var(--aria-bg)] px-4 py-6 text-[var(--aria-ink)] md:px-6 lg:px-8"
    >
      <div className="mx-auto max-w-7xl rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-6 shadow-sm">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wide text-[var(--aria-primary)]">
              Image Create Agent
            </p>
            <h1 className="mt-1 text-2xl font-semibold">图片创作</h1>
          </div>
          <a
            className="rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm font-semibold hover:bg-[var(--aria-panel-muted)]"
            href="/workbench"
          >
            返回工作台
          </a>
        </div>
        <p className="mt-4 text-sm text-[var(--aria-ink-muted)]">
          {sessionId ? `当前会话：${sessionId}` : "选择或创建会话以开始图片创作。"}
        </p>
      </div>
    </main>
  );
}
