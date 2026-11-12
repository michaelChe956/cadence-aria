import { Cpu, LoaderCircle, PlugZap, RotateCcw, Save, Settings, Zap } from "lucide-react";
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
  type ImageVendorConnectivityState,
} from "../../state/image-create-store";
import { CustomSelect } from "./CustomSelect";

/**
 * 统一控件外观：与 CustomSelect 触发器同款（边框色 / 圆角 / 内边距 /
 * focus ring / hover 行为），输入框、textarea 与下拉因此视觉一致。
 */
const controlClassName =
  "block w-full min-h-11 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3.5 py-2.5 text-base font-medium text-[var(--aria-ink)] shadow-[inset_0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60 sm:text-sm";

/** 单个供应方的编辑态。 */
type VendorDraft = {
  baseUrl: string;
  apiKey: string;
  apiKeyAction: ApiKeyAction;
  /** 模型清单原文，一行一个 id。 */
  modelsText: string;
};

type VendorDrafts = Record<ImageVendor, VendorDraft>;

/** 各家的 Base URL 填写示例：只填到主机根路径。 */
const BASE_URL_EXAMPLES: Record<ImageVendor, string> = {
  gpt: "https://api.openai.com",
  minimax: "https://api.minimax.io",
};

/** Base URL 填写说明：一句话讲清「填到哪」。 */
const BASE_URL_HINT =
  "只填到主机根路径，不要带 /v1 或 /v1/images；系统会自动拼成 /v1/images/generations。";

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

/**
 * 分区状态 chip：按编辑态判断凭据是否齐全（与 store 的“已配置”同口径：
 * base_url 与 key 都在位），用户改完 URL / key 即时反映。
 */
function isVendorConfigured(draft: VendorDraft): boolean {
  return (
    draft.baseUrl.trim() !== "" &&
    draft.apiKeyAction !== "clear" &&
    draft.apiKey.trim() !== ""
  );
}

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const loadSettings = useImageCreateStore((state) => state.loadSettings);
  const saveSettings = useImageCreateStore((state) => state.saveSettings);
  const testVendorConnectivity = useImageCreateStore(
    (state) => state.testVendorConnectivity,
  );
  const connectivity = useImageCreateStore((state) => state.connectivity);
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
      },
      minimax: {
        baseUrl: settings.vendors.minimax.base_url,
        apiKey: settings.vendors.minimax.api_key_masked,
        apiKeyAction: "retain",
        modelsText: settings.vendors.minimax.models.join("\n"),
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
      const baseUrlChanged = trimmedBaseUrl !== originalVendor.base_url;
      const modelsChanged =
        models.length !== originalVendor.models.length ||
        models.some((model, i) => model !== originalVendor.models[i]);
      const keyChanged = draft.apiKeyAction !== "retain";
      // 完全未改动的供应方不进入请求。
      if (!baseUrlChanged && !modelsChanged && !keyChanged) {
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

  /**
   * 用当前草稿值发起连通性探测：只发探测请求，不生成图片、不计费。
   * key 用草稿里的明文；沿用已保存 key 时传空串，由后端取已保存的那把。
   */
  function handleTestConnectivity(vendor: ImageVendor) {
    const draft = drafts?.[vendor];
    if (!draft) {
      return;
    }
    void testVendorConnectivity(vendor, {
      baseUrl: draft.baseUrl,
      apiKey: draft.apiKeyAction === "replace" ? draft.apiKey : "",
      models: parseModels(draft.modelsText),
    });
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
          <div className="space-y-5">
            {IMAGE_VENDORS.map((vendor) =>
              original && drafts ? (
                <VendorSection
                  key={vendor}
                  vendor={vendor}
                  original={original.vendors[vendor]}
                  draft={drafts[vendor]}
                  disabled={controlsDisabled}
                  connectivity={connectivity[vendor]}
                  onChange={(next) => updateVendor(vendor, next)}
                  onTestConnectivity={() => handleTestConnectivity(vendor)}
                />
              ) : null,
            )}

            {defaults ? (
              <div className="border-t border-[var(--aria-line)] pt-5">
                <fieldset disabled={controlsDisabled}>
                  <legend className="text-sm font-semibold text-[var(--aria-ink)]">
                    默认生成参数
                  </legend>
                  <div className="mt-3 grid gap-3 sm:grid-cols-2">
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
              </div>
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
            className="cursor-pointer rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2.5 text-sm font-semibold text-[var(--aria-ink-muted)] shadow-[0_2px_8px_rgba(23,32,42,0.08)] transition-colors duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
          >
            取消
          </button>
          <button
            type="submit"
            disabled={controlsDisabled}
            className="inline-flex cursor-pointer items-center gap-2 rounded-xl border border-[#4338ca] bg-[var(--aria-primary)] px-4 py-2.5 text-sm font-semibold text-white shadow-[0_2px_8px_rgba(23,32,42,0.08)] transition-colors duration-200 hover:bg-[#4338ca] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
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
  connectivity,
  onChange,
  onTestConnectivity,
}: {
  vendor: ImageVendor;
  original: MaskedVendorSettings;
  draft: VendorDraft;
  disabled: boolean;
  connectivity: ImageVendorConnectivityState;
  onChange: (draft: VendorDraft) => void;
  onTestConnectivity: () => void;
}) {
  const label = IMAGE_VENDOR_LABELS[vendor];
  const configured = isVendorConfigured(draft);
  const probing = connectivity.phase === "testing";
  // API Key 三态只用下方一行文字表达，措辞与实际动作一致。
  const apiKeyState =
    draft.apiKeyAction === "clear"
      ? "保存后将清除当前 key"
      : draft.apiKeyAction === "replace"
        ? "保存后将替换当前 key"
        : draft.apiKey.trim() === ""
          ? "尚未配置 key，填写后保存生效"
          : `已保存 ${draft.apiKey}，留空保持不变`;

  return (
    <section
      aria-label={label}
      className="rounded-2xl border-2 border-[var(--aria-border-muted)] bg-[var(--aria-panel)] p-4 shadow-[0_2px_8px_rgba(23,32,42,0.08)]"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2.5">
          <span
            aria-hidden="true"
            className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
          >
            <VendorIcon vendor={vendor} />
          </span>
          <h3 className="text-base font-semibold text-[var(--aria-ink)]">
            {label}
          </h3>
        </div>
        <span
          className={
            configured
              ? "inline-flex items-center rounded-full border border-[var(--aria-success)] bg-[var(--aria-success-soft)] px-2.5 py-1 text-xs font-semibold text-[var(--aria-ink)]"
              : "inline-flex items-center rounded-full border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2.5 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]"
          }
        >
          {configured ? "已配置" : "未配置"}
        </span>
      </div>

      <div className="mt-3">
        <label
          htmlFor={`${vendor}-base-url`}
          className="text-sm font-semibold text-[var(--aria-ink)]"
        >
          Base URL
        </label>
        <input
          id={`${vendor}-base-url`}
          aria-label={`${label} base_url`}
          placeholder={BASE_URL_EXAMPLES[vendor]}
          value={draft.baseUrl}
          disabled={disabled}
          onChange={(event) =>
            onChange({ ...draft, baseUrl: event.target.value })
          }
          className={`${controlClassName} mt-1.5`}
        />
        <p className="mt-1.5 text-xs text-[var(--aria-ink-muted)]">
          {BASE_URL_HINT}
        </p>
        <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
          必须使用 HTTPS；本地调试可用 localhost。
        </p>
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={onTestConnectivity}
            disabled={disabled || probing}
            className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-1.5 text-xs font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {probing ? (
              <LoaderCircle
                aria-hidden="true"
                className="h-3.5 w-3.5 motion-safe:animate-spin"
              />
            ) : (
              <PlugZap aria-hidden="true" className="h-3.5 w-3.5" />
            )}
            {probing ? "测试中…" : "测试连通"}
          </button>
          <span className="text-xs text-[var(--aria-ink-muted)]">
            只测连通性，不生成图片、不计费
          </span>
        </div>
      </div>

      <div className="mt-3">
        <label
          htmlFor={`${vendor}-api-key`}
          className="text-sm font-semibold text-[var(--aria-ink)]"
        >
          API Key
        </label>
        <div className="mt-1.5 flex items-center gap-2">
          <input
            id={`${vendor}-api-key`}
            aria-label={`${label} API Key`}
            value={draft.apiKeyAction === "clear" ? "" : draft.apiKey}
            placeholder={
              draft.apiKeyAction === "clear" ? "保存后将清除" : undefined
            }
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
            className={`${controlClassName} min-w-0 flex-1`}
          />
          <button
            type="button"
            aria-label="清除 key"
            disabled={disabled}
            onClick={() =>
              onChange({ ...draft, apiKeyAction: "clear", apiKey: "" })
            }
            className="shrink-0 cursor-pointer rounded-lg px-2 py-1.5 text-xs font-semibold text-[var(--aria-danger)] transition-colors duration-200 hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
          >
            清除
          </button>
        </div>
        <p className="mt-1.5 text-xs text-[var(--aria-ink-muted)]">
          {apiKeyState}
        </p>
      </div>

      <div className="mt-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <label
            htmlFor={`${vendor}-models`}
            className="text-sm font-semibold text-[var(--aria-ink)]"
          >
            模型清单
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
            className="inline-flex cursor-pointer items-center gap-1 rounded-full border border-transparent bg-[var(--aria-primary-soft)] px-3 py-1 text-xs font-semibold text-[var(--aria-primary)] transition-colors duration-200 hover:border-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
          >
            <RotateCcw aria-hidden="true" className="h-3.5 w-3.5" />
            恢复推荐清单
          </button>
        </div>
        <textarea
          id={`${vendor}-models`}
          aria-label={`${label} 模型清单`}
          value={draft.modelsText}
          rows={8}
          spellCheck={false}
          disabled={disabled}
          onChange={(event) =>
            onChange({ ...draft, modelsText: event.target.value })
          }
          className={`${controlClassName} mt-1.5 min-h-[10rem] resize-y font-mono`}
        />
        <p className="mt-1.5 text-xs text-[var(--aria-ink-muted)]">
          一行一个模型 id，保存时自动去空行与首尾空白。
        </p>
      </div>

      <ConnectivityResult state={connectivity} />
    </section>
  );
}

/** 连通性探测结果：内联展示结论、每一步详情与网关未列出的模型。 */
function ConnectivityResult({ state }: { state: ImageVendorConnectivityState }) {
  const result = state.result;
  if (!result && !state.error) {
    return null;
  }
  return (
    <div className="mt-3 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-xs text-[var(--aria-ink)]">
      {state.error ? (
        <p className="font-semibold text-[var(--aria-danger)]">{state.error}</p>
      ) : null}
      {result ? (
        <>
          <p
            className={
              result.ok
                ? "font-semibold text-[var(--aria-success)]"
                : "font-semibold text-[var(--aria-danger)]"
            }
          >
            {result.message}
          </p>
          <ul className="mt-1.5 space-y-1">
            {result.steps.map((step) => (
              <li key={step.key}>
                <span className="font-semibold">{step.label}</span>
                <span className="ml-1 text-[var(--aria-ink-muted)]">
                  {step.detail}
                </span>
              </li>
            ))}
          </ul>
          {result.missing_models.length > 0 ? (
            <p className="mt-1.5 font-semibold text-[var(--aria-danger)]">
              网关未列出这些模型：{result.missing_models.join("、")}
            </p>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

/** 分区图标：SVG 图标（非 emoji），仅作装饰。 */
function VendorIcon({ vendor }: { vendor: ImageVendor }) {
  return vendor === "gpt" ? (
    <Cpu className="h-4 w-4" />
  ) : (
    <Zap className="h-4 w-4" />
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
