import type { ReactNode } from "react";

export type WorkbenchSurfaceProps = {
  header: ReactNode;
  statusBar?: ReactNode;
  alert?: ReactNode;
  main: ReactNode;
  aside?: ReactNode;
  mainLabel?: string;
  asideLabel?: string;
};

export function WorkbenchSurface({
  header,
  statusBar,
  alert,
  main,
  aside,
  mainLabel = "工作台主区域",
  asideLabel = "工作台检查器",
}: WorkbenchSurfaceProps) {
  return (
    <div className="min-h-screen bg-[var(--aria-bg)] text-[var(--aria-ink)]">
      <header
        role="banner"
        className="sticky top-0 z-30 border-b border-[var(--aria-line)] bg-[var(--aria-panel)]/96 px-4 py-2 shadow-sm backdrop-blur md:px-6 lg:px-8"
      >
        {header}
      </header>
      {statusBar ? (
        <section className="border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-2 md:px-6 lg:px-8">
          {statusBar}
        </section>
      ) : null}
      {alert ? (
        <div className="border-b border-red-200 bg-red-50 px-4 py-2 text-sm font-semibold text-red-800 md:px-6 lg:px-8">
          {alert}
        </div>
      ) : null}
      <main
        aria-label={mainLabel}
        className={
          aside
            ? "grid min-h-[calc(100vh-4rem)] grid-cols-1 gap-4 px-4 py-4 md:px-6 lg:grid-cols-[minmax(0,1fr)_26rem] lg:px-8 xl:grid-cols-[minmax(0,1fr)_30rem]"
            : "min-h-[calc(100vh-4rem)] px-4 py-4 md:px-6 lg:px-8"
        }
      >
        <div className="min-w-0">{main}</div>
        {aside ? (
          <aside aria-label={asideLabel} className="min-w-0 space-y-4">
            {aside}
          </aside>
        ) : null}
      </main>
    </div>
  );
}
