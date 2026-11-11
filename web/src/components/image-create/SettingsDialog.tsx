import { KeyRound, LoaderCircle, RotateCcw, Save, Settings } from "lucide-react";
import { DialogShell } from "../shared/DialogShell";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  IMAGE_BACKGROUND_OPTIONS,
  IMAGE_OUTPUT_FORMAT_OPTIONS,
  IMAGE_QUALITY_OPTIONS,
  IMAGE_SIZE_OPTIONS,
  IMAGE_VENDOR_LABELS,
  IMAGE_VENDORS,
  type ApiKeyAction,
  type DefaultImageParams,
  type ImageVendor,
  type MaskedSettings,
  type MaskedVendorSettings,
  type SettingsUpdateRequest,
  type VendorSettingsUpdate,
} from "../../api/types/image-create";
import {
  useImageCreateStore,
  validateImageCreateBaseUrl,
} from "../../state/image-create-store";
import { CustomSelect } from "./CustomSelect";

const inputClassName =
  "mt-1.5 block w-full rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3.5 py-2.5 text-sm font-normal text-[var(--aria-ink)] shadow-[inset_0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:bg-[var(--aria-panel-muted)] disabled:opacity-60";

/** 单个供应方的编辑态。 */
type VendorDraft = {
  baseUrl: string;
  apiKey: string;
  apiKeyAction: ApiKeyAction;
  /** 模型清单原文，一行一个 id。 */
  modelsText: string;
  defaultModel: string;
};

type VendorDrafts = Record<ImageVendor, VendorDraft>;

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

/** 解析模型清单：逐行去首尾空白，丢弃空行。 */
function parseModels(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
}

/** 轮转模型：原选项仍在清单中则保留，否则自动落到清单首项；清单为空则为空。 */
function effectiveDefaultModel(draft: VendorDraft): string {
  const models = parseModels(draft.modelsText);
  if (models.includes(draft.defaultModel)) {
    return draft.defaultModel;
  }
  return models[0] ?? "";
}

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const loadSettings = useImageCreateStore((state) => state.loadSettings);
  const saveSettings = useImageCreateStore((state) => state.saveSettings);
  const [original, setOriginal] = useState<MaskedSettings | null>(null);
  const [drafts, setDrafts] = useState<VendorDrafts | null>(null);
  const [defaults, setDefaults] = useState<DefaultImageParams | null>(null);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const submittingRef = useRef(false);

  function hydrate(settings: MaskedSettings) {
    setOriginal(settings);
    setDrafts({
      gpt: {
        baseUrl: settings.vendors.gpt.base_url,
        apiKey: settings.vendors.gpt.api_key_masked,
        apiKeyAction: "retain",
        modelsText: settings.vendors.gpt.models.join("\n"),
        defaultModel: settings.vendors.gpt.default_model,
      },
      minimax: {
        baseUrl: settings.vendors.minimax.base_url,
        apiKey: settings.vendors.minimax.api_key_masked,
        apiKeyAction: "retain",
        modelsText: settings.vendors.minimax.models.join("\n"),
        defaultModel: settings.vendors.minimax.default_model,
      },
    });
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

  function updateVendor(vendor: ImageVendor, next: VendorDraft) {
    setDrafts((current) =>
      current ? { ...current, [vendor]: next } : current,
    );
    setError(null);
    setSuccess(null);
  }

  function updateDefault<K extends keyof DefaultImageParams>(
    key: K,
    value: DefaultImageParams[K],
  ) {
    setDefaults((current) => (current ? { ...current, [key]: value } : current));
    setError(null);
    setSuccess(null);
  }

  function buildUpdate(): SettingsUpdateRequest | null {
    if (!original || !defaults || !drafts) {
      setError("设置尚未加载完成");
      return null;
    }

    const vendors: Partial<Record<ImageVendor, VendorSettingsUpdate>> = {};
    for (const vendor of IMAGE_VENDORS) {
      const originalVendor = original.vendors[vendor];
      const draft = drafts[vendor];
      const trimmedBaseUrl = draft.baseUrl.trim();
      const models = parseModels(draft.modelsText);
      const defaultModel = effectiveDefaultModel(draft);
      const baseUrlChanged = trimmedBaseUrl !== originalVendor.base_url;
      const modelsChanged =
        models.length !== originalVendor.models.length ||
        models.some((model, i) => model !== originalVendor.models[i]);
      const defaultChanged = defaultModel !== originalVendor.default_model;
      const keyChanged = draft.apiKeyAction !== "retain";
      // 完全未改动的供应方不进入请求。
      if (!baseUrlChanged && !modelsChanged && !defaultChanged && !keyChanged) {
        continue;
      }

      if (baseUrlChanged && trimmedBaseUrl !== "") {
        try {
          validateImageCreateBaseUrl(trimmedBaseUrl);
        } catch (reason) {
          setError(settingsErrorMessage(reason, "base_url 无效"));
          return null;
        }
      }

      const update: VendorSettingsUpdate = {
        api_key_action: draft.apiKeyAction,
      };
      if (baseUrlChanged) {
        update.base_url = trimmedBaseUrl;
      }
      if (draft.apiKeyAction === "replace") {
        const replacement = draft.apiKey.trim();
        if (!replacement) {
          setError("请输入新的 API Key，或使用“清除 key”按钮");
          return null;
        }
        update.api_key = replacement;
      }
      if (modelsChanged) {
        update.models = models;
      }
      if (defaultChanged) {
        update.default_model = defaultModel;
      }
      vendors[vendor] = update;
    }

    const request: SettingsUpdateRequest = {};
    if (Object.keys(vendors).length > 0) {
      request.vendors = vendors;
    }
    if (!defaultsEqual(defaults, original.defaults)) {
      request.defaults = defaults;
    }
    return request;
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

  const controlsDisabled = loading || submitting || !original || !defaults || !drafts;

  return (
    <DialogShell
      label="图片创作设置"
      onClose={onClose}
      closeLabel="关闭"
      widthClass="max-w-2xl"
      icon={<Settings aria-hidden="true" className="h-4 w-4" />}
      description="API Key 仅展示脱敏值；不修改时会安全保留原值。"
    >
      <form onSubmit={handleSubmit}>
        {loading ? (
          <div className="flex items-center justify-center gap-2 rounded-xl bg-[var(--aria-panel-muted)] py-10 text-sm text-[var(--aria-ink-muted)]">
            <LoaderCircle aria-hidden="true" className="h-4 w-4 motion-safe:animate-spin" />
            正在加载设置…
          </div>
        ) : (
          <div className="space-y-5 rounded-xl bg-[var(--aria-panel-muted)] p-4">
            {IMAGE_VENDORS.map((vendor) =>
              original && drafts ? (
                <VendorSection
                  key={vendor}
                  vendor={vendor}
                  original={original.vendors[vendor]}
                  draft={drafts[vendor]}
                  disabled={controlsDisabled}
                  onChange={(next) => updateVendor(vendor, next)}
                />
              ) : null,
            )}

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
    </DialogShell>
  );
}

function VendorSection({
  vendor,
  original,
  draft,
  disabled,
  onChange,
}: {
  vendor: ImageVendor;
  original: MaskedVendorSettings;
  draft: VendorDraft;
  disabled: boolean;
  onChange: (draft: VendorDraft) => void;
}) {
  const label = IMAGE_VENDOR_LABELS[vendor];
  const models = parseModels(draft.modelsText);
  const defaultModel = effectiveDefaultModel(draft);

  return (
    <section
      aria-label={label}
      className="rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3"
    >
      <h3 className="text-sm font-semibold text-[var(--aria-ink)]">{label}</h3>
      <label className="mt-2.5 block text-sm font-semibold text-[var(--aria-ink)]">
        {label} base_url
        <input
          aria-label={`${label} base_url`}
          value={draft.baseUrl}
          disabled={disabled}
          onChange={(event) =>
            onChange({ ...draft, baseUrl: event.target.value })
          }
          className={inputClassName}
        />
        <span className="mt-1 block text-xs font-normal text-[var(--aria-ink-muted)]">
          必须使用 HTTPS；本地调试可使用 HTTP localhost 或 loopback IP。
        </span>
      </label>

      <div className="mt-3">
        <label className="block text-sm font-semibold text-[var(--aria-ink)]">
          {label} API Key
          <input
            aria-label={`${label} API Key`}
            value={draft.apiKeyAction === "clear" ? "" : draft.apiKey}
            placeholder={draft.apiKeyAction === "clear" ? "保存后将清除" : undefined}
            autoComplete="off"
            disabled={disabled}
            onChange={(event) => {
              const value = event.target.value;
              onChange({
                ...draft,
                apiKey: value,
                apiKeyAction:
                  value === original.api_key_masked ? "retain" : "replace",
              });
            }}
            className={inputClassName}
          />
        </label>
        <div className="mt-2.5 flex flex-wrap items-center justify-between gap-2">
          <span className="text-xs text-[var(--aria-ink-muted)]">
            {draft.apiKeyAction === "retain"
              ? "当前 key 将保持不变"
              : draft.apiKeyAction === "clear"
                ? "保存后清除当前 key"
                : "保存后替换当前 key"}
          </span>
          <button
            type="button"
            disabled={disabled}
            onClick={() =>
              onChange({ ...draft, apiKeyAction: "clear", apiKey: "" })
            }
            className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-[var(--aria-danger)] bg-[var(--aria-panel)] px-3 py-2 text-xs font-semibold text-[var(--aria-danger)] transition-all duration-200 hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
          >
            <KeyRound aria-hidden="true" className="h-3.5 w-3.5" />
            清除 key
          </button>
        </div>
      </div>

      <div className="mt-3">
        <label className="block text-sm font-semibold text-[var(--aria-ink)]">
          {label} 模型清单
          <textarea
            aria-label={`${label} 模型清单`}
            value={draft.modelsText}
            rows={4}
            spellCheck={false}
            disabled={disabled}
            onChange={(event) =>
              onChange({ ...draft, modelsText: event.target.value })
            }
            className={`${inputClassName} min-h-24 resize-y font-mono`}
          />
          <span className="mt-1 block text-xs font-normal text-[var(--aria-ink-muted)]">
            一行一个模型 id；保存时自动去除空行与首尾空白。
          </span>
        </label>
        <button
          type="button"
          disabled={disabled}
          onClick={() =>
            onChange({
              ...draft,
              modelsText: original.recommended_models.join("\n"),
            })
          }
          className="mt-2 inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-xs font-semibold text-[var(--aria-ink)] transition-all duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
        >
          <RotateCcw aria-hidden="true" className="h-3.5 w-3.5" />
          恢复推荐清单
        </button>
      </div>

      <div className="mt-3">
        <CustomSelect
          label={`${label} 轮转模型`}
          aria-label={`${label} 轮转模型`}
          value={defaultModel}
          options={models}
          disabled={disabled}
          onChange={(value) => onChange({ ...draft, defaultModel: value })}
        />
        <span className="mt-1 block text-xs text-[var(--aria-ink-muted)]">
          自动轮转到该家时使用的模型；清单变化后自动落到首项。
        </span>
      </div>
    </section>
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
    <CustomSelect
      label={label}
      value={value}
      options={options}
      onChange={(nextValue) => onChange(nextValue as T)}
    />
  );
}
