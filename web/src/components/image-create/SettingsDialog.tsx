import { KeyRound, LoaderCircle, Save, Settings, X } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  IMAGE_BACKGROUND_OPTIONS,
  IMAGE_OUTPUT_FORMAT_OPTIONS,
  IMAGE_QUALITY_OPTIONS,
  IMAGE_SIZE_OPTIONS,
  type ApiKeyAction,
  type DefaultImageParams,
  type MaskedSettings,
  type SettingsUpdateRequest,
} from "../../api/types/image-create";
import {
  useImageCreateStore,
  validateImageCreateBaseUrl,
} from "../../state/image-create-store";

const inputClassName =
  "mt-1.5 block w-full rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3.5 py-2.5 text-sm font-normal text-[var(--aria-ink)] shadow-[inset_0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:bg-[var(--aria-panel-muted)] disabled:opacity-60";

function settingsErrorMessage(reason: unknown, fallback: string): string {
  return reason instanceof Error ? reason.message : fallback;
}

function defaultsEqual(left: DefaultImageParams, right: DefaultImageParams) {
  return (
    left.size === right.size &&
    left.quality === right.quality &&
    left.background === right.background &&
    left.output_format === right.output_format
  );
}

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const loadSettings = useImageCreateStore((state) => state.loadSettings);
  const saveSettings = useImageCreateStore((state) => state.saveSettings);
  const [original, setOriginal] = useState<MaskedSettings | null>(null);
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [apiKeyAction, setApiKeyAction] = useState<ApiKeyAction>("retain");
  const [defaults, setDefaults] = useState<DefaultImageParams | null>(null);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const submittingRef = useRef(false);

  function hydrate(settings: MaskedSettings) {
    setOriginal(settings);
    setBaseUrl(settings.base_url);
    setApiKey(settings.api_key_masked);
    setApiKeyAction("retain");
    setDefaults(settings.defaults);
  }

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);
    void loadSettings()
      .then(() => {
        if (!active) {
          return;
        }
        const settings = useImageCreateStore.getState().settings;
        if (!settings) {
          setError("未能加载图片创作设置");
          return;
        }
        hydrate(settings);
      })
      .catch((reason: unknown) => {
        if (active) {
          setError(settingsErrorMessage(reason, "加载图片创作设置失败"));
        }
      })
      .finally(() => {
        if (active) {
          setLoading(false);
        }
      });
    return () => {
      active = false;
    };
  }, [loadSettings]);

  function updateDefault<K extends keyof DefaultImageParams>(
    key: K,
    value: DefaultImageParams[K],
  ) {
    setDefaults((current) => (current ? { ...current, [key]: value } : current));
    setError(null);
    setSuccess(null);
  }

  function buildUpdate(): SettingsUpdateRequest | null {
    if (!original || !defaults) {
      setError("设置尚未加载完成");
      return null;
    }

    const trimmedBaseUrl = baseUrl.trim();
    try {
      validateImageCreateBaseUrl(trimmedBaseUrl);
    } catch (reason) {
      setError(settingsErrorMessage(reason, "base_url 无效"));
      return null;
    }

    const update: SettingsUpdateRequest = {
      api_key_action: apiKeyAction,
    };
    if (trimmedBaseUrl !== original.base_url) {
      update.base_url = trimmedBaseUrl;
    }
    if (!defaultsEqual(defaults, original.defaults)) {
      update.defaults = defaults;
    }

    if (apiKeyAction === "replace") {
      const replacement = apiKey.trim();
      if (!replacement || replacement === original.api_key_masked) {
        setError("请输入新的 API Key，或使用“清除 key”按钮");
        return null;
      }
      update.api_key = replacement;
    }
    return update;
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submittingRef.current) {
      return;
    }
    const update = buildUpdate();
    if (!update) {
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    setSuccess(null);
    try {
      await saveSettings(update);
      await loadSettings();
      const settings = useImageCreateStore.getState().settings;
      if (!settings) {
        throw new Error("保存成功，但刷新设置失败");
      }
      hydrate(settings);
      setSuccess("设置已保存");
    } catch (reason) {
      setError(settingsErrorMessage(reason, "保存图片创作设置失败"));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  const controlsDisabled = loading || submitting || !original || !defaults;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/40 p-4 backdrop-blur-sm">
      <form
        role="dialog"
        aria-label="图片创作设置"
        aria-modal="true"
        onSubmit={handleSubmit}
        className="max-h-[calc(100vh-2rem)] w-full max-w-2xl overflow-y-auto rounded-2xl border border-white/80 bg-[var(--aria-panel)] p-5 shadow-[0_24px_64px_rgba(15,23,42,0.22)]"
      >
        <div className="mb-5 flex items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
              <Settings aria-hidden="true" className="h-5 w-5" />
            </span>
            <div>
              <h2 className="text-base font-semibold text-[var(--aria-ink)]">
                图片创作设置
              </h2>
              <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
                API Key 仅展示脱敏值；不修改时会安全保留原值。
              </p>
            </div>
          </div>
          <button
            type="button"
            aria-label="关闭"
            onClick={onClose}
            className="inline-flex h-9 w-9 cursor-pointer items-center justify-center rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-all duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
          >
            <X aria-hidden="true" className="h-4 w-4" />
          </button>
        </div>

        {loading ? (
          <div className="flex items-center justify-center gap-2 rounded-xl bg-[var(--aria-panel-muted)] py-10 text-sm text-[var(--aria-ink-muted)]">
            <LoaderCircle aria-hidden="true" className="h-4 w-4 motion-safe:animate-spin" />
            正在加载设置…
          </div>
        ) : (
          <div className="space-y-5 rounded-xl bg-[var(--aria-panel-muted)] p-4">
            <label className="block text-sm font-semibold text-[var(--aria-ink)]">
              base_url
              <input
                aria-label="base_url"
                value={baseUrl}
                disabled={controlsDisabled}
                onChange={(event) => {
                  setBaseUrl(event.target.value);
                  setError(null);
                  setSuccess(null);
                }}
                className={inputClassName}
              />
              <span className="mt-1 block text-xs font-normal text-[var(--aria-ink-muted)]">
                必须使用 HTTPS；本地调试可使用 HTTP localhost 或 loopback IP。
              </span>
            </label>

            <div>
              <label className="block text-sm font-semibold text-[var(--aria-ink)]">
                API Key
                <input
                  aria-label="API Key"
                  value={apiKeyAction === "clear" ? "" : apiKey}
                  placeholder={apiKeyAction === "clear" ? "保存后将清除" : undefined}
                  autoComplete="off"
                  disabled={controlsDisabled}
                  onChange={(event) => {
                    const value = event.target.value;
                    setApiKey(value);
                    setApiKeyAction(
                      value === original?.api_key_masked ? "retain" : "replace",
                    );
                    setError(null);
                    setSuccess(null);
                  }}
                  className={inputClassName}
                />
              </label>
              <div className="mt-2.5 flex flex-wrap items-center justify-between gap-2">
                <span className="text-xs text-[var(--aria-ink-muted)]">
                  {apiKeyAction === "retain"
                    ? "当前 key 将保持不变"
                    : apiKeyAction === "clear"
                      ? "保存后清除当前 key"
                      : "保存后替换当前 key"}
                </span>
                <button
                  type="button"
                  disabled={controlsDisabled}
                  onClick={() => {
                    setApiKeyAction("clear");
                    setApiKey("");
                    setError(null);
                    setSuccess(null);
                  }}
                  className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-[var(--aria-danger)] bg-[var(--aria-panel)] px-3 py-2 text-xs font-semibold text-[var(--aria-danger)] transition-all duration-200 hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
                >
                  <KeyRound aria-hidden="true" className="h-3.5 w-3.5" />
                  清除 key
                </button>
              </div>
            </div>

            {defaults ? (
              <fieldset disabled={controlsDisabled}>
                <legend className="text-sm font-semibold text-[var(--aria-ink)]">
                  默认生成参数
                </legend>
                <div className="mt-2.5 grid gap-3 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 sm:grid-cols-2">
                  <SettingsSelect
                    label="默认尺寸"
                    value={defaults.size}
                    options={IMAGE_SIZE_OPTIONS}
                    onChange={(value) => updateDefault("size", value)}
                  />
                  <SettingsSelect
                    label="默认质量"
                    value={defaults.quality}
                    options={IMAGE_QUALITY_OPTIONS}
                    onChange={(value) => updateDefault("quality", value)}
                  />
                  <SettingsSelect
                    label="默认背景"
                    value={defaults.background}
                    options={IMAGE_BACKGROUND_OPTIONS}
                    onChange={(value) => updateDefault("background", value)}
                  />
                  <SettingsSelect
                    label="默认输出格式"
                    value={defaults.output_format}
                    options={IMAGE_OUTPUT_FORMAT_OPTIONS}
                    onChange={(value) => updateDefault("output_format", value)}
                  />
                </div>
              </fieldset>
            ) : null}

            {error ? (
              <p role="alert" className="rounded-lg border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
                {error}
              </p>
            ) : null}
            {success ? (
              <p role="status" className="rounded-lg border border-[var(--aria-success)] bg-[var(--aria-success-soft)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
                {success}
              </p>
            ) : null}
          </div>
        )}

        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="cursor-pointer rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2.5 text-sm font-semibold text-[var(--aria-ink-muted)] shadow-sm transition-all duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
          >
            取消
          </button>
          <button
            type="submit"
            disabled={controlsDisabled}
            className="inline-flex cursor-pointer items-center gap-2 rounded-xl bg-gradient-to-b from-[var(--aria-primary)] to-[#0e7490] px-4 py-2.5 text-sm font-semibold text-white shadow-[0_4px_12px_rgba(8,145,178,0.24)] transition-all duration-200 hover:translate-y-px hover:shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0.5 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {submitting ? (
              <LoaderCircle aria-hidden="true" className="h-4 w-4 motion-safe:animate-spin" />
            ) : (
              <Save aria-hidden="true" className="h-4 w-4" />
            )}
            {submitting ? "保存中…" : "保存设置"}
          </button>
        </div>
      </form>
    </div>
  );
}

function SettingsSelect<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: readonly T[];
  onChange: (value: T) => void;
}) {
  return (
    <label className="block text-sm font-semibold text-[var(--aria-ink)]">
      {label}
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value as T)}
        className={inputClassName}
      >
        {options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    </label>
  );
}
