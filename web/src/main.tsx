import React from "react";
import ReactDOM from "react-dom/client";
import "./styles.css";

export function AppShell() {
  return (
    <div className="min-h-screen bg-[#eef3f4] text-ink">
      <header
        role="banner"
        className="flex h-12 items-center justify-between border-b border-line bg-white px-4"
      >
        <strong>Aria Web</strong>
        <span className="text-sm text-slate-600">single workspace</span>
      </header>
      <div className="grid min-h-[calc(100vh-3rem)] grid-cols-[18rem_minmax(0,1fr)_24rem]">
        <nav aria-label="Node flow" className="border-r border-line bg-panel p-3">
          <span className="text-xs font-semibold uppercase text-slate-500">Flow</span>
        </nav>
        <main className="p-4">
          <h1 className="text-xl font-semibold">Node Workspace</h1>
        </main>
        <aside className="border-l border-line bg-white p-3">
          <span className="text-sm font-medium">Evidence</span>
        </aside>
      </div>
    </div>
  );
}

const root = document.getElementById("root");

if (root) {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <AppShell />
    </React.StrictMode>,
  );
}
