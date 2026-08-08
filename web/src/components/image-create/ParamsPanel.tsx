import { ChevronDown, LoaderCircle, SlidersHorizontal, Wand2 } from "lucide-react";
import { useEffect, useState } from "react";
import {
  IMAGE_BACKGROUND_OPTIONS,
  IMAGE_INPUT_FIDELITY_OPTIONS,
  IMAGE_OUTPUT_FORMAT_OPTIONS,
  IMAGE_QUALITY_OPTIONS,
  IMAGE_SIZE_OPTIONS,
  type ImageBackground,
  type ImageInputFidelity,
  type ImageOutputFormat,
  type ImageQuality,
  type ImageSize,
} from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { CustomSelect } from "./CustomSelect";

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
  const referenceImage = useImageCreateStore((state) => state.referenceImage);
  const currentSession = useImageCreateStore((state) => state.currentSession);
  const isBusy = useImageCreateStore((state) => state.isBusy);
  const setParams = useImageCreateStore((state) => state.setParams);
  const generate = useImageCreateStore((state) => state.generate);
  const [paramsExpanded, setParamsExpanded] = useState(false);
  const elapsed = useElapsedSeconds(isBusy);

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
            gpt-image-2 生成通常需要 1-3 分钟，请耐心等待，<strong>不要重复点击生成或刷新页面</strong>。
          </p>
        </div>
      ) : null}
      <div
        id="image-create-parameter-fields"
        className={`mt-4 gap-3 rounded-xl bg-[var(--aria-panel-muted)] p-3 sm:grid-cols-2 lg:grid ${
          paramsExpanded ? "grid" : "hidden"
        }`}
      >
        <ParameterSelect
          label="尺寸"
          value={params.size}
          options={IMAGE_SIZE_OPTIONS}
          disabled={isBusy}
          onChange={(value) => setParams({ size: value as ImageSize })}
        />
        <ParameterSelect
          label="质量"
          value={params.quality}
          options={IMAGE_QUALITY_OPTIONS}
          disabled={isBusy}
          onChange={(value) => setParams({ quality: value as ImageQuality })}
        />
        <ParameterSelect
          label="背景"
          value={params.background}
          options={IMAGE_BACKGROUND_OPTIONS}
          disabled={isBusy}
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
        {referenceImage ? (
          <ParameterSelect
            label="参考图保真度"
            value={params.input_fidelity ?? "low"}
            options={IMAGE_INPUT_FIDELITY_OPTIONS}
            disabled={isBusy}
            onChange={(value) =>
              setParams({ input_fidelity: value as ImageInputFidelity })
            }
          />
        ) : null}
      </div>
      <button
        type="button"
        onClick={() => {
          generate().catch(() => {});
        }}
        disabled={isBusy || !currentSession || !params.prompt.trim()}
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
