import { useEffect, useState } from "react";
import {
  getSpecGenerationMode,
  type SpecGenerationMode,
} from "./api/groupChat";
import type { CodingAttemptAddress } from "./api/types";
import { IssueLifecycleWorkbench } from "./components/lifecycle/IssueLifecycleWorkbench";
import { SpecGenerationSettings } from "./components/settings/SpecGenerationSettings";
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
  const [specGenerationMode, setSpecGenerationMode] =
    useState<SpecGenerationMode>("pipeline");

  useEffect(() => {
    let cancelled = false;
    getSpecGenerationMode()
      .then((mode) => {
        if (!cancelled) {
          setSpecGenerationMode(mode);
        }
      })
      .catch(() => {
        // 设置读取失败时保留流水线默认值，避免阻塞看板。
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
      <a
        href="/image-create"
        className="fixed bottom-4 right-4 z-40 rounded-full border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-4 py-2 text-sm font-semibold text-white shadow-lg transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
      >
        图片创作
      </a>
      <div className="fixed right-4 top-4 z-40">
        <button
          type="button"
          onClick={() => setSettingsOpen((open) => !open)}
          aria-expanded={settingsOpen}
          className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-xs font-semibold text-[var(--aria-ink)] shadow-sm"
        >
          设置
        </button>
        {settingsOpen ? (
          <div className="absolute right-0 mt-2 w-[min(30rem,calc(100vw-2rem))]">
            <SpecGenerationSettings
              onModeChange={setSpecGenerationMode}
            />
          </div>
        ) : null}
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
