import { useEffect, useState } from "react";
import {
  getSpecGenerationMode,
  setSpecGenerationMode,
  type SpecGenerationMode,
} from "../../api/groupChat";

export function SpecGenerationSettings({
  onModeChange,
}: {
  onModeChange?: (mode: SpecGenerationMode) => void;
}) {
  const [mode, setMode] = useState<SpecGenerationMode>("pipeline");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    getSpecGenerationMode()
      .then((currentMode) => {
        if (!cancelled) {
          setMode(currentMode);
        }
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : "读取 Spec 生成模式失败");
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  async function handleChange(nextMode: SpecGenerationMode) {
    if (nextMode === mode || saving) {
      return;
    }
    const previousMode = mode;
    setMode(nextMode);
    setSaving(true);
    setError(null);
    try {
      const savedMode = await setSpecGenerationMode(nextMode);
      setMode(savedMode);
      onModeChange?.(savedMode);
    } catch (reason) {
      setMode(previousMode);
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
      <fieldset className="mt-4 grid gap-2 sm:grid-cols-2" disabled={loading || saving}>
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
      {loading ? <p className="mt-3 text-xs text-[var(--aria-ink-muted)]">正在读取设置…</p> : null}
      {saving ? <p className="mt-3 text-xs text-[var(--aria-ink-muted)]">正在保存…</p> : null}
      {error ? <p role="alert" className="mt-3 text-xs text-[var(--aria-danger)]">{error}</p> : null}
    </section>
  );
}
