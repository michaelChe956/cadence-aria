import { useEffect } from "react";
import { ChatPane } from "../components/image-create/ChatPane";
import { ParamsPanel } from "../components/image-create/ParamsPanel";
import { PromptBlock } from "../components/image-create/PromptBlock";
import { ReferenceImageUpload } from "../components/image-create/ReferenceImageUpload";
import { SessionList } from "../components/image-create/SessionList";
import { useImageCreateStore } from "../state/image-create-store";

export function ImageCreatePage({ sessionId }: { sessionId?: string }) {
  const error = useImageCreateStore((state) => state.error);
  const loadSessions = useImageCreateStore((state) => state.loadSessions);
  const loadSettings = useImageCreateStore((state) => state.loadSettings);
  const openSession = useImageCreateStore((state) => state.openSession);
  const disconnect = useImageCreateStore((state) => state.disconnect);

  useEffect(() => {
    void loadSessions().catch(() => undefined);
    void loadSettings().catch(() => undefined);
    return () => disconnect();
  }, [disconnect, loadSessions, loadSettings]);

  useEffect(() => {
    if (sessionId) {
      void openSession(sessionId).catch(() => undefined);
    }
  }, [openSession, sessionId]);

  return (
    <main
      aria-label="图片创作"
      className="min-h-screen bg-[var(--aria-bg)] px-4 py-5 text-[var(--aria-ink)] md:px-6"
    >
      <div className="mx-auto max-w-[1600px]">
        <header className="mb-4 flex flex-wrap items-center justify-between gap-3">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wide text-[var(--aria-primary)]">
              Image Create Agent
            </p>
            <h1 className="mt-1 text-2xl font-semibold">图片创作</h1>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              title="设置弹窗将在 Task 12 实现"
              className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-sm font-semibold hover:bg-[var(--aria-panel-muted)]"
            >
              设置
            </button>
            <a
              className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-sm font-semibold hover:bg-[var(--aria-panel-muted)]"
              href="/workbench"
            >
              返回工作台
            </a>
          </div>
        </header>
        {error ? (
          <div
            role="alert"
            className="mb-4 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm font-semibold text-red-700"
          >
            {error}
          </div>
        ) : null}
        <div className="grid min-h-[calc(100vh-8rem)] gap-4 lg:grid-cols-[18rem_minmax(0,1fr)]">
          <SessionList />
          <div className="grid min-w-0 gap-4 xl:grid-cols-[minmax(0,1fr)_22rem]">
            <ChatPane />
            <div className="space-y-4">
              <PromptBlock />
              <ReferenceImageUpload />
              <ParamsPanel />
            </div>
          </div>
        </div>
      </div>
    </main>
  );
}
