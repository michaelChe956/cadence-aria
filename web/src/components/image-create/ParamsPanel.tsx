import { ChevronDown, LoaderCircle, SlidersHorizontal, Wand2 } from "lucide-react";
import { useEffect, useState } from "react";
import {
  IMAGE_BACKGROUND_OPTIONS,
  IMAGE_OUTPUT_FORMAT_OPTIONS,
  IMAGE_QUALITY_OPTIONS,
  IMAGE_SIZE_OPTIONS,
  IMAGE_VENDOR_LABELS,
  IMAGE_VENDORS,
  type ImageBackground,
  type ImageOutputFormat,
  type ImageQuality,
  type ImageSize,
  type ImageVendor,
  type MaskedSettings,
} from "../../api/types/image-create";
import {
  availableImageVendors,
  configuredImageVendors,
  useImageCreateStore,
} from "../../state/image-create-store";
import { CustomSelect, type CustomSelectOption } from "./CustomSelect";

/** 没有可用供应方时的提示。 */
const NO_VENDOR_HINT = "请先在设置里配置并启用至少一个供应方";

function useElapsedSeconds(active: boolean): number {
  const [seconds, setSeconds] = useState(0);
  useEffect(() => {
    if (!active) {
      setSeconds(0);
      return;
    }
    setSeconds(0);
    const id = window.setInterval(() => setSeconds((s) => s + 1), 1000);
    return () => window.clearInterval(id);
  }, [active]);
  return seconds;
}

export function ParamsPanel() {
  const params = useImageCreateStore((state) => state.params);
  const entries = useImageCreateStore((state) => state.entries);
  const settings = useImageCreateStore((state) => state.settings);
  const referenceImage = useImageCreateStore((state) => state.referenceImage);
  const currentSession = useImageCreateStore((state) => state.currentSession);
  const isBusy = useImageCreateStore((state) => state.isBusy);
  const setParams = useImageCreateStore((state) => state.setParams);
  const generate = useImageCreateStore((state) => state.generate);
  const [paramsExpanded, setParamsExpanded] = useState(false);
  const elapsed = useElapsedSeconds(isBusy);

  // 只列出已配置凭据且清单非空的供应方的模型 id。
  const configuredVendors = configuredImageVendors(settings);
  const availableVendors = availableImageVendors(settings);
  const modelOptions: CustomSelectOption[] = availableVendors.flatMap((vendor) =>
    (settings?.vendors[vendor].models ?? []).map((model) => ({
      value: model,
      label: model,
    })),
  );
  const selectedVendor = modelVendor(params.model, settings, availableVendors);
  const modelDisplay = params.model || "未选择模型";

  // MiniMax 不支持质量与背景；显式选择 MiniMax 模型时这两个参数置灰。
  const minimaxSelected = selectedVendor === "minimax";
  const vendorParamsDisabled = isBusy || minimaxSelected;
  const vendorParamsHint = minimaxSelected ? "MiniMax 不支持该参数" : null;

  // 会话内尚无生成图、或已选择上传参考图时，不携带上一张图。
  const hasGeneratedImage = entries.some((entry) => entry.type === "generation_image");
  const previousDisabled = isBusy || !hasGeneratedImage || Boolean(referenceImage);
  const previousHint = referenceImage
    ? "已使用上传的参考图"
    : !hasGeneratedImage
      ? "首张图片将直接生成"
      : null;

  return (
    <section
      aria-labelledby="image-create-params-heading"
      className="rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-[0_2px_8px_rgba(15,23,42,0.04),0_10px_28px_rgba(15,23,42,0.06)] transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md"
    >
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
            <SlidersHorizontal aria-hidden="true" className="h-4 w-4" />
          </span>
          <h2 id="image-create-params-heading" className="text-base font-semibold">
            生成参数
          </h2>
        </div>
        <div className="flex items-center gap-2">
          {isBusy ? (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-[var(--aria-primary-soft)] px-2.5 py-1 text-xs font-semibold text-[var(--aria-ink)]">
              <LoaderCircle aria-hidden="true" className="h-3.5 w-3.5 motion-safe:animate-spin" />
              处理中
            </span>
          ) : null}
          <button
            type="button"
            aria-label={paramsExpanded ? "收起生成参数" : "展开生成参数"}
            aria-expanded={paramsExpanded}
            aria-controls="image-create-parameter-fields"
            onClick={() => setParamsExpanded((expanded) => !expanded)}
            className="inline-flex min-h-11 min-w-11 cursor-pointer items-center justify-center rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-all duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 lg:hidden"
          >
            <ChevronDown
              aria-hidden="true"
              className={`h-5 w-5 transition-transform duration-200 motion-reduce:transition-none ${
                paramsExpanded ? "rotate-180" : ""
              }`}
            />
          </button>
        </div>
      </div>
      {isBusy ? (
        <div
          role="status"
          aria-live="polite"
          className="mt-4 rounded-xl border border-[var(--aria-primary)]/40 bg-[var(--aria-primary-soft)] px-3.5 py-3 text-sm text-[var(--aria-ink)] shadow-sm"
        >
          <div className="flex items-center gap-2 font-semibold">
            <span className="relative flex h-3 w-3">
              <span className="absolute inline-flex h-full w-full rounded-full bg-[var(--aria-primary)] opacity-50 motion-safe:animate-ping" />
              <span className="relative inline-flex h-3 w-3 rounded-full bg-[var(--aria-primary)]" />
            </span>
            正在生成图片…已等待 {elapsed} 秒
          </div>
          <p className="mt-1.5 text-xs leading-5">
            图片生成通常需要 1-3 分钟，请耐心等待，<strong>不要重复点击生成或刷新页面</strong>。
          </p>
        </div>
      ) : null}
      <div
        id="image-create-parameter-fields"
        className={`mt-4 gap-3 rounded-xl bg-[var(--aria-panel-muted)] p-3 sm:grid-cols-2 lg:grid ${
          paramsExpanded ? "grid" : "hidden"
        }`}
      >
        <div className="sm:col-span-2">
          <CustomSelect
            label="模型"
            value={params.model}
            displayValue={modelDisplay}
            options={modelOptions}
            disabled={isBusy}
            onChange={(value) => setParams({ model: value })}
          />
          {settings
            ? IMAGE_VENDORS.filter(
                (vendor) => !configuredVendors.includes(vendor),
              ).map((vendor) => (
                <p
                  key={vendor}
                  className="mt-1.5 text-xs text-[var(--aria-ink-muted)]"
                >
                  {IMAGE_VENDOR_LABELS[vendor]} 未配置，可在设置中添加
                </p>
              ))
            : null}
          {settings && modelOptions.length === 0 ? (
            <p className="mt-1.5 text-xs text-[var(--aria-ink-muted)]">
              {NO_VENDOR_HINT}
            </p>
          ) : null}
        </div>
        <div className="sm:col-span-2">
          <label className="flex min-h-11 items-center gap-2.5 text-sm font-semibold text-[var(--aria-ink)]">
            <input
              type="checkbox"
              aria-label="基于上一张图继续修改"
              checked={params.use_previous}
              disabled={previousDisabled}
              onChange={(event) =>
                setParams({ use_previous: event.target.checked })
              }
              className="h-5 w-5 cursor-pointer accent-[var(--aria-primary)] disabled:cursor-not-allowed disabled:opacity-50"
            />
            基于上一张图继续修改
          </label>
          {previousHint ? (
            <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
              {previousHint}
            </p>
          ) : null}
        </div>
        <ParameterSelect
          label="尺寸"
          value={params.size}
          options={IMAGE_SIZE_OPTIONS}
          disabled={isBusy}
          onChange={(value) => setParams({ size: value as ImageSize })}
        />
        <div>
          <ParameterSelect
            label="质量"
            value={params.quality}
            options={IMAGE_QUALITY_OPTIONS}
            disabled={vendorParamsDisabled}
            onChange={(value) => setParams({ quality: value as ImageQuality })}
          />
          {vendorParamsHint ? (
            <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
              {vendorParamsHint}
            </p>
          ) : null}
        </div>
        <ParameterSelect
          label="背景"
          value={params.background}
          options={IMAGE_BACKGROUND_OPTIONS}
          disabled={vendorParamsDisabled}
          onChange={(value) => setParams({ background: value as ImageBackground })}
        />
        <ParameterSelect
          label="输出格式"
          value={params.output_format}
          options={IMAGE_OUTPUT_FORMAT_OPTIONS}
          disabled={isBusy}
          onChange={(value) =>
            setParams({ output_format: value as ImageOutputFormat })
          }
        />
      </div>
      <button
        type="button"
        onClick={() => {
          generate().catch(() => {});
        }}
        disabled={
          isBusy || !currentSession || !params.prompt.trim() || modelOptions.length === 0
        }
        className="mt-4 inline-flex min-h-11 w-full cursor-pointer items-center justify-center gap-2 rounded-xl bg-gradient-to-b from-[var(--aria-primary)] to-[#0e7490] px-4 py-3 text-sm font-semibold text-white shadow-[0_5px_16px_rgba(8,145,178,0.26)] transition-all duration-200 hover:translate-y-px hover:shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0.5 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {isBusy ? (
          <LoaderCircle aria-hidden="true" className="h-4 w-4 motion-safe:animate-spin" />
        ) : (
          <Wand2 aria-hidden="true" className="h-4 w-4" />
        )}
        {isBusy ? `生成中…（${elapsed}s）` : "生成图片"}
      </button>
      {!currentSession ? (
        <p className="mt-2.5 text-center text-xs text-[var(--aria-ink-muted)]">请先选择或创建会话。</p>
      ) : !params.prompt.trim() ? (
        <p className="mt-2.5 text-center text-xs text-[var(--aria-ink-muted)]">请先填写建议提示词。</p>
      ) : null}
    </section>
  );
}

/** 模型 id 归属的供应方：只依据可用供应方的清单判定。 */
function modelVendor(
  model: string,
  settings: MaskedSettings | null,
  availableVendors: ImageVendor[],
): ImageVendor | null {
  if (!settings || !model) {
    return null;
  }
  for (const vendor of availableVendors) {
    if (settings.vendors[vendor].models.includes(model)) {
      return vendor;
    }
  }
  return null;
}

function ParameterSelect({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  options: readonly string[];
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <CustomSelect
      label={label}
      value={value}
      options={options}
      disabled={disabled}
      onChange={onChange}
    />
  );
}
