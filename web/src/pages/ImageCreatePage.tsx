import { ArrowLeft, Settings, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";
import { ChatPane } from "../components/image-create/ChatPane";
import { ParamsPanel } from "../components/image-create/ParamsPanel";
import { PromptBlock } from "../components/image-create/PromptBlock";
import { ReferenceImageUpload } from "../components/image-create/ReferenceImageUpload";
import { SessionList } from "../components/image-create/SessionList";
import { SettingsDialog } from "../components/image-create/SettingsDialog";
import { useImageCreateStore } from "../state/image-create-store";

export function ImageCreatePage({ sessionId }: { sessionId?: string }) {
  const [settingsOpen, setSettingsOpen] = useState(false);
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
      className="min-h-screen bg-[var(--aria-bg)] px-4 py-6 text-[var(--aria-ink)] md:px-6 lg:py-8"
    >
      <div className="mx-auto max-w-[1600px]">
        <header className="relative mb-6 overflow-hidden rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-5 py-5 shadow-[0_2px_8px_rgba(15,23,42,0.04),0_12px_32px_rgba(15,23,42,0.06)] md:px-6">
          <div className="pointer-events-none absolute -right-16 -top-20 h-48 w-48 rounded-full bg-[var(--aria-primary-soft)] opacity-80 blur-2xl" />
          <div className="relative flex flex-wrap items-center justify-between gap-4">
            <div className="flex items-center gap-4">
              <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-gradient-to-br from-[var(--aria-primary-soft)] to-white text-[var(--aria-primary)] shadow-[inset_0_1px_1px_rgba(255,255,255,0.9),0_6px_16px_rgba(8,145,178,0.14)]">
                <Sparkles aria-hidden="true" className="h-6 w-6" />
              </div>
              <div>
                <p className="text-xs font-semibold uppercase tracking-[0.16em] text-[var(--aria-primary)]">
                  Image Create Agent
                </p>
                <h1 className="mt-1 text-2xl font-semibold tracking-tight md:text-3xl">
                  图片创作
                </h1>
                <p className="mt-1.5 max-w-2xl text-sm leading-6 text-[var(--aria-ink-muted)]">
                  与创作 Agent 协作打磨提示词、配置生成参数，快速产出专业图片。
                </p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => setSettingsOpen(true)}
                className="inline-flex cursor-pointer items-center gap-2 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3.5 py-2.5 text-sm font-semibold shadow-sm transition-all duration-200 hover:-translate-y-0.5 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0"
              >
                <Settings aria-hidden="true" className="h-4 w-4" />
                设置
              </button>
              <a
                className="inline-flex cursor-pointer items-center gap-2 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3.5 py-2.5 text-sm font-semibold shadow-sm transition-all duration-200 hover:-translate-y-0.5 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0"
                href="/workbench"
              >
                <ArrowLeft aria-hidden="true" className="h-4 w-4" />
                返回工作台
              </a>
            </div>
          </div>
        </header>
        {error ? (
          <div
            role="alert"
            className="mb-5 rounded-xl border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-4 py-3 text-sm font-semibold text-[var(--aria-ink)] shadow-sm"
          >
            {error}
          </div>
        ) : null}
        <div className="grid min-h-[calc(100vh-12rem)] gap-5 lg:grid-cols-[18rem_minmax(0,1fr)]">
          <SessionList />
          <div className="grid min-w-0 gap-5 xl:grid-cols-[minmax(0,1fr)_22rem]">
            <ChatPane />
            <div className="space-y-5">
              <PromptBlock />
              <ReferenceImageUpload />
              <ParamsPanel />
            </div>
          </div>
        </div>
      </div>
      {settingsOpen ? (
        <SettingsDialog onClose={() => setSettingsOpen(false)} />
      ) : null}
    </main>
  );
}
