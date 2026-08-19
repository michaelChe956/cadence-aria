import { useState } from "react";
import {
  setSpecGenerationMode,
  type SpecGenerationMode,
} from "../../api/groupChat";

/** 本地缓存键：与 AppShell 共享，保证首屏不闪。 */
export const SPEC_GENERATION_MODE_CACHE_KEY = "aria:spec-generation-mode";

export function readCachedSpecGenerationMode(): SpecGenerationMode {
  try {
    const cached = window.localStorage.getItem(SPEC_GENERATION_MODE_CACHE_KEY);
    if (cached === "group_chat" || cached === "pipeline") {
      return cached;
    }
  } catch {
    // localStorage 不可用时静默回退默认值。
  }
  return "pipeline";
}

export function writeCachedSpecGenerationMode(mode: SpecGenerationMode) {
  try {
    window.localStorage.setItem(SPEC_GENERATION_MODE_CACHE_KEY, mode);
  } catch {
    // 忽略写入失败：仅影响下次首屏闪动，不影响正确性。
  }
}

export function SpecGenerationSettings({
  mode,
  onModeChange,
}: {
  /** 由 AppShell 受控传入，面板自身不再发读取请求。 */
  mode: SpecGenerationMode;
  onModeChange?: (mode: SpecGenerationMode) => void;
}) {
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleChange(nextMode: SpecGenerationMode) {
    if (nextMode === mode || saving) {
      return;
    }
    setSaving(true);
    setError(null);
    // 乐观更新：先切换 UI 与本地缓存，失败时回滚。
    onModeChange?.(nextMode);
    writeCachedSpecGenerationMode(nextMode);
    try {
      const savedMode = await setSpecGenerationMode(nextMode);
      onModeChange?.(savedMode);
      writeCachedSpecGenerationMode(savedMode);
    } catch (reason) {
      onModeChange?.(mode);
      writeCachedSpecGenerationMode(mode);
      setError(reason instanceof Error ? reason.message : "保存 Spec 生成模式失败");
    } finally {
      setSaving(false);
    }
  }

  return (
    <section
      aria-label="Spec 生成模式设置"
      className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4"
    >
      <div>
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">Spec 生成模式</h2>
        <p className="mt-1 text-xs leading-5 text-[var(--aria-ink-muted)]">
          选择新建 Spec 时使用流水线或群聊工作台。
        </p>
      </div>
      <fieldset className="mt-4 grid gap-2 sm:grid-cols-2" disabled={saving}>
        <legend className="sr-only">Spec 生成模式</legend>
        <div className="flex items-start gap-2 rounded-md border border-[var(--aria-line)] p-3 text-sm">
          <input
            id="spec-generation-mode-pipeline"
            type="radio"
            name="spec-generation-mode"
            value="pipeline"
            checked={mode === "pipeline"}
            onChange={() => void handleChange("pipeline")}
          />
          <div>
            <label htmlFor="spec-generation-mode-pipeline" className="block cursor-pointer font-semibold text-[var(--aria-ink)]">流水线模式</label>
            <span className="mt-1 block text-xs text-[var(--aria-ink-muted)]">默认的 Story / Design 工作流。</span>
          </div>
        </div>
        <div className="flex items-start gap-2 rounded-md border border-[var(--aria-line)] p-3 text-sm">
          <input
            id="spec-generation-mode-group-chat"
            type="radio"
            name="spec-generation-mode"
            value="group_chat"
            checked={mode === "group_chat"}
            onChange={() => void handleChange("group_chat")}
          />
          <div>
            <label htmlFor="spec-generation-mode-group-chat" className="block cursor-pointer font-semibold text-[var(--aria-ink)]">群聊模式</label>
            <span className="mt-1 block text-xs text-[var(--aria-ink-muted)]">在多人群聊中讨论并定稿 Spec。</span>
          </div>
        </div>
      </fieldset>
      {saving ? <p className="mt-3 text-xs text-[var(--aria-ink-muted)]">正在保存…</p> : null}
      {error ? <p role="alert" className="mt-3 text-xs text-[var(--aria-danger)]">{error}</p> : null}
    </section>
  );
}
