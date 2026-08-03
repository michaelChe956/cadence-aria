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
  "mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-sm font-normal text-[var(--aria-ink)] transition-colors hover:border-[var(--aria-line-strong)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)] disabled:opacity-60";

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
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4">
      <form
        role="dialog"
        aria-label="图片创作设置"
        aria-modal="true"
        onSubmit={handleSubmit}
        className="max-h-[calc(100vh-2rem)] w-full max-w-2xl overflow-y-auto rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <div>
            <h2 className="text-base font-semibold text-[var(--aria-ink)]">
              图片创作设置
            </h2>
            <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
              API Key 仅展示脱敏值；不修改时会安全保留原值。
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            关闭
          </button>
        </div>

        {loading ? (
          <p className="py-8 text-center text-sm text-[var(--aria-ink-muted)]">
            正在加载设置…
          </p>
        ) : (
          <div className="space-y-4">
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
              <div className="mt-2 flex flex-wrap items-center justify-between gap-2">
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
                  className="rounded-md border border-[var(--aria-danger)] px-3 py-1.5 text-xs font-semibold text-[var(--aria-danger)] transition-colors hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)] disabled:opacity-60"
                >
                  清除 key
                </button>
              </div>
            </div>

            {defaults ? (
              <fieldset disabled={controlsDisabled}>
                <legend className="text-sm font-semibold text-[var(--aria-ink)]">
                  默认生成参数
                </legend>
                <div className="mt-2 grid gap-3 sm:grid-cols-2">
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
              <p role="alert" className="text-sm font-semibold text-[var(--aria-danger)]">
                {error}
              </p>
            ) : null}
            {success ? (
              <p role="status" className="text-sm font-semibold text-[var(--aria-success)]">
                {success}
              </p>
            ) : null}
          </div>
        )}

        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink-muted)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            取消
          </button>
          <button
            type="submit"
            disabled={controlsDisabled}
            className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:opacity-60"
          >
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
