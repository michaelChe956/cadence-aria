import { ArrowLeft, Menu, Settings, Sparkles } from "lucide-react";
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
  const [sessionsOpen, setSessionsOpen] = useState(false);
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
      className="min-h-screen bg-[var(--aria-bg)] px-4 py-4 text-[var(--aria-ink)] sm:py-6 md:px-6 lg:py-8"
    >
      <div className="mx-auto max-w-[1600px]">
        <header className="relative mb-4 overflow-hidden rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-4 shadow-[0_2px_8px_rgba(15,23,42,0.04),0_12px_32px_rgba(15,23,42,0.06)] sm:mb-6 sm:px-5 sm:py-5 md:px-6">
          <div className="pointer-events-none absolute -right-16 -top-20 h-48 w-48 rounded-full bg-[var(--aria-primary-soft)] opacity-80 blur-2xl" />
          <div className="relative flex items-center justify-between gap-3 sm:gap-4">
            <div className="flex min-w-0 items-center gap-3 sm:gap-4">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-[var(--aria-primary-soft)] to-white text-[var(--aria-primary)] shadow-[inset_0_1px_1px_rgba(255,255,255,0.9),0_6px_16px_rgba(8,145,178,0.14)] sm:h-12 sm:w-12 sm:rounded-2xl">
                <Sparkles aria-hidden="true" className="h-5 w-5 sm:h-6 sm:w-6" />
              </div>
              <div className="min-w-0">
                <p className="hidden text-xs font-semibold uppercase tracking-[0.16em] text-[var(--aria-primary)] sm:block">
                  Image Create Agent
                </p>
                <h1 className="truncate text-xl font-semibold tracking-tight sm:mt-1 sm:text-2xl md:text-3xl">
                  图片创作
                </h1>
                <p className="mt-1.5 hidden max-w-2xl text-sm leading-6 text-[var(--aria-ink-muted)] sm:block">
                  与创作 Agent 协作打磨提示词、配置生成参数，快速产出专业图片。
                </p>
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <button
                type="button"
                aria-label="设置"
                title="设置"
                onClick={() => setSettingsOpen(true)}
                className="inline-flex min-h-11 min-w-11 cursor-pointer items-center justify-center gap-2 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-sm font-semibold shadow-sm transition-all duration-200 hover:-translate-y-0.5 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0 sm:px-3.5"
              >
                <Settings aria-hidden="true" className="h-4 w-4" />
                <span className="hidden sm:inline">设置</span>
              </button>
              <a
                aria-label="返回工作台"
                title="返回工作台"
                className="inline-flex min-h-11 min-w-11 cursor-pointer items-center justify-center gap-2 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-sm font-semibold shadow-sm transition-all duration-200 hover:-translate-y-0.5 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0 sm:px-3.5"
                href="/workbench"
              >
                <ArrowLeft aria-hidden="true" className="h-4 w-4" />
                <span className="hidden sm:inline">返回工作台</span>
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
        <button
          type="button"
          aria-label="打开会话列表"
          aria-expanded={sessionsOpen}
          aria-controls="image-create-session-drawer"
          onClick={() => setSessionsOpen(true)}
          className="mb-4 inline-flex min-h-11 cursor-pointer items-center gap-2 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 text-sm font-semibold shadow-sm transition-all duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 lg:hidden"
        >
          <Menu aria-hidden="true" className="h-5 w-5" />
          会话
        </button>
        {sessionsOpen ? (
          <button
            type="button"
            aria-label="关闭会话列表遮罩"
            onClick={() => setSessionsOpen(false)}
            className="fixed inset-0 z-40 cursor-default bg-slate-950/45 backdrop-blur-sm lg:hidden"
          />
        ) : null}
        <div
          data-testid="image-create-workspace"
          className="min-h-[calc(100vh-12rem)] lg:grid lg:grid-cols-[18rem_minmax(0,1fr)] lg:gap-5"
        >
          <div
            id="image-create-session-drawer"
            data-testid="image-create-session-drawer"
            className={`fixed inset-y-0 left-0 z-50 w-[min(20rem,calc(100vw-3rem))] p-3 transition-transform duration-300 motion-reduce:transition-none lg:static lg:z-auto lg:w-auto lg:translate-x-0 lg:p-0 ${
              sessionsOpen ? "translate-x-0" : "-translate-x-full"
            }`}
          >
            <SessionList
              onClose={() => setSessionsOpen(false)}
              onSessionSelect={() => setSessionsOpen(false)}
            />
          </div>
          <div
            data-testid="image-create-main-area"
            className="grid min-w-0 gap-4 lg:gap-5 xl:grid-cols-[minmax(0,1fr)_22rem]"
          >
            <ChatPane />
            <div className="space-y-4 lg:space-y-5">
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
