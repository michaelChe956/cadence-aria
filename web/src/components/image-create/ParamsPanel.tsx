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

const selectClassName =
  "mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-sm text-[var(--aria-ink)] transition-colors hover:border-[var(--aria-line-strong)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)] disabled:opacity-60";

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
  const elapsed = useElapsedSeconds(isBusy);

  return (
    <section
      aria-labelledby="image-create-params-heading"
      className="rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-sm"
    >
      <div className="flex items-center justify-between gap-3">
        <h2 id="image-create-params-heading" className="text-base font-semibold">
          生成参数
        </h2>
        {isBusy ? (
          <span className="text-xs font-semibold text-[var(--aria-primary)]">处理中</span>
        ) : null}
      </div>
      {isBusy ? (
        <div
          role="status"
          aria-live="polite"
          className="mt-3 rounded-lg border border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] px-3 py-2 text-sm text-[var(--aria-ink)]"
        >
          <div className="flex items-center gap-2 font-semibold">
            <span className="inline-block h-2 w-2 animate-pulse rounded-full bg-[var(--aria-primary)]"></span>
            正在生成图片…已等待 {elapsed} 秒
          </div>
          <p className="mt-1 text-xs">
            gpt-image-2 生成通常需要 1-3 分钟，请耐心等待，<strong>不要重复点击生成或刷新页面</strong>。
          </p>
        </div>
      ) : null}
      <div className="mt-3 grid gap-3 sm:grid-cols-2">
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
        className="mt-4 w-full rounded-md bg-[var(--aria-primary)] px-4 py-2.5 text-sm font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {isBusy ? `生成中…（${elapsed}s）` : "生成图片"}
      </button>
      {!currentSession ? (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">请先选择或创建会话。</p>
      ) : !params.prompt.trim() ? (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">请先填写建议提示词。</p>
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
    <label className="block text-sm font-semibold text-[var(--aria-ink)]">
      {label}
      <select
        aria-label={label}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        className={selectClassName}
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
