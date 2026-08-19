import { Settings } from "lucide-react";
import { useEffect, useState } from "react";
import {
  getSpecGenerationMode,
  type SpecGenerationMode,
} from "./api/groupChat";
import type { CodingAttemptAddress } from "./api/types";
import { IssueLifecycleWorkbench } from "./components/lifecycle/IssueLifecycleWorkbench";
import {
  SpecGenerationSettings,
  readCachedSpecGenerationMode,
  writeCachedSpecGenerationMode,
} from "./components/settings/SpecGenerationSettings";
import { WhatsNewDialog } from "./components/whats-new/WhatsNewDialog";
import { useWhatsNew } from "./whats-new/useWhatsNew";

export function AppShell({
  focusEntityKey,
  onDrawerFocusChange,
  onOpenWorkspace,
  onOpenCodingWorkspace,
  onOpenGroupChat,
}: {
  focusEntityKey?: string | null;
  onDrawerFocusChange?: (entityKey: string | null) => void;
  onOpenWorkspace?: (sessionId: string) => void;
  onOpenCodingWorkspace?: (address: CodingAttemptAddress) => void;
  onOpenGroupChat?: (sessionId: string) => void;
}) {
  const whatsNew = useWhatsNew();
  const [settingsOpen, setSettingsOpen] = useState(false);
  // 首屏用 localStorage 缓存同步初始化，避免异步读取返回前后看板整体切换造成的闪动。
  const [specGenerationMode, setSpecGenerationMode] =
    useState<SpecGenerationMode>(() => readCachedSpecGenerationMode());

  useEffect(() => {
    let cancelled = false;
    getSpecGenerationMode()
      .then((mode) => {
        if (!cancelled) {
          setSpecGenerationMode(mode);
          writeCachedSpecGenerationMode(mode);
        }
      })
      .catch(() => {
        // 设置读取失败时保留本地缓存值，避免阻塞看板。
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const previousScrollRestoration = window.history.scrollRestoration;
    window.history.scrollRestoration = "manual";
    window.scrollTo({ top: 0, left: 0, behavior: "auto" });

    return () => {
      window.history.scrollRestoration = previousScrollRestoration;
    };
  }, []);

  return (
    <div className="relative">
      {whatsNew.open && whatsNew.entry ? (
        <WhatsNewDialog entry={whatsNew.entry} onClose={whatsNew.close} />
      ) : null}
      <div className="fixed bottom-4 right-4 z-40 flex items-center gap-2">
        <div className="relative">
          <button
            type="button"
            onClick={() => setSettingsOpen((open) => !open)}
            aria-label="设置"
            aria-expanded={settingsOpen}
            title="设置"
            className="inline-flex cursor-pointer items-center justify-center gap-2 rounded-full border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2 text-sm font-semibold text-[var(--aria-ink)] shadow-lg transition-all duration-200 hover:-translate-y-0.5 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0"
          >
            <Settings aria-hidden="true" className="h-4 w-4" />
            <span className="hidden sm:inline">设置</span>
          </button>
          {settingsOpen ? (
            <div className="absolute bottom-full right-0 mb-2 w-[min(30rem,calc(100vw-2rem))]">
              <SpecGenerationSettings
                mode={specGenerationMode}
                onModeChange={setSpecGenerationMode}
              />
            </div>
          ) : null}
        </div>
        <a
          href="/image-create"
          className="rounded-full border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-4 py-2 text-sm font-semibold text-white shadow-lg transition-all duration-200 hover:-translate-y-0.5 hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0"
        >
          图片创作
        </a>
      </div>
      <IssueLifecycleWorkbench
        focusEntityKey={focusEntityKey}
        onDrawerFocusChange={onDrawerFocusChange}
        onOpenWorkspace={onOpenWorkspace}
        onOpenCodingWorkspace={onOpenCodingWorkspace}
        onOpenGroupChat={onOpenGroupChat}
        specGenerationMode={specGenerationMode}
      />
    </div>
  );
}
