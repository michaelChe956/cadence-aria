import { ImagePlus, Trash2, Upload } from "lucide-react";
import { useEffect, useId, useRef, useState, type ChangeEvent } from "react";
import { useImageCreateStore } from "../../state/image-create-store";

export const REFERENCE_IMAGE_MAX_BYTES = 10 * 1024 * 1024;
export const REFERENCE_IMAGE_MIME_TYPES = [
  "image/png",
  "image/jpeg",
  "image/webp",
] as const;

export function ReferenceImageUpload() {
  const inputId = useId();
  const inputRef = useRef<HTMLInputElement | null>(null);
  const referenceImage = useImageCreateStore((state) => state.referenceImage);
  const params = useImageCreateStore((state) => state.params);
  const isBusy = useImageCreateStore((state) => state.isBusy);
  const setParams = useImageCreateStore((state) => state.setParams);
  const setReferenceImage = useImageCreateStore((state) => state.setReferenceImage);
  const [error, setError] = useState<string | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);

  useEffect(() => {
    if (!referenceImage) {
      setPreviewUrl(null);
      return;
    }
    const url = URL.createObjectURL(referenceImage);
    setPreviewUrl(url);
    return () => URL.revokeObjectURL(url);
  }, [referenceImage]);

  function handleFileChange(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0] ?? null;
    if (!file) {
      return;
    }
    if (!REFERENCE_IMAGE_MIME_TYPES.includes(file.type as (typeof REFERENCE_IMAGE_MIME_TYPES)[number])) {
      setError("仅支持 PNG、JPEG、WebP 格式的参考图");
      event.target.value = "";
      return;
    }
    if (file.size > REFERENCE_IMAGE_MAX_BYTES) {
      setError("参考图大小不能超过 10MB");
      event.target.value = "";
      return;
    }
    setError(null);
    if (params.input_fidelity === null) {
      setParams({ input_fidelity: "low" });
    }
    setReferenceImage(file);
  }

  function removeReference() {
    setError(null);
    setReferenceImage(null);
    if (inputRef.current) {
      inputRef.current.value = "";
    }
  }

  return (
    <section
      aria-labelledby="image-create-reference-heading"
      className="rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-[0_2px_8px_rgba(15,23,42,0.04),0_10px_28px_rgba(15,23,42,0.06)] transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md"
    >
      <div className="flex items-center gap-2.5">
        <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
          <ImagePlus aria-hidden="true" className="h-4 w-4" />
        </span>
        <div>
          <h2 id="image-create-reference-heading" className="text-base font-semibold">
            参考图
          </h2>
          <p className="mt-0.5 text-xs text-[var(--aria-ink-muted)]">
            PNG、JPEG、WebP · 不超过 10MB
          </p>
        </div>
      </div>
      {referenceImage && previewUrl ? (
        <div className="mt-4 overflow-hidden rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] shadow-inner">
          <div className="p-2">
            <img
              src={previewUrl}
              alt="参考图预览"
              className="max-h-48 w-full rounded-lg object-contain"
            />
          </div>
          <div className="flex items-center justify-between gap-3 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2.5">
            <span className="min-w-0 truncate text-xs text-[var(--aria-ink-muted)]">
              {referenceImage.name}
            </span>
            <button
              type="button"
              onClick={removeReference}
              disabled={isBusy}
              title={isBusy ? "处理中不可修改参考图" : undefined}
              className="inline-flex min-h-11 shrink-0 cursor-pointer items-center gap-1.5 rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-xs font-semibold text-[var(--aria-danger)] transition-all duration-200 hover:border-[var(--aria-danger)] hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Trash2 aria-hidden="true" className="h-3.5 w-3.5" />
              移除参考图
            </button>
          </div>
        </div>
      ) : (
        <label
          htmlFor={isBusy ? undefined : inputId}
          aria-disabled={isBusy}
          title={isBusy ? "处理中不可修改参考图" : undefined}
          className={`mt-4 flex flex-col items-center rounded-xl border border-dashed border-[var(--aria-line-strong)] bg-[var(--aria-panel-muted)] px-4 py-7 text-center shadow-inner transition-all duration-200 ${
            isBusy
              ? "cursor-not-allowed opacity-60"
              : "cursor-pointer hover:-translate-y-0.5 hover:border-[var(--aria-primary)] hover:bg-[var(--aria-primary-soft)] hover:shadow-sm focus-within:ring-2 focus-within:ring-[var(--aria-primary)] focus-within:ring-offset-2"
          }`}
        >
          <span className="flex h-11 w-11 items-center justify-center rounded-xl bg-[var(--aria-panel)] text-[var(--aria-primary)] shadow-sm">
            <Upload aria-hidden="true" className="h-5 w-5" />
          </span>
          <span className="mt-3 text-sm font-semibold">选择一张参考图</span>
          <span className="mt-1 text-xs text-[var(--aria-ink-muted)]">点击浏览本地文件</span>
        </label>
      )}
      <input
        ref={inputRef}
        id={inputId}
        aria-label="上传参考图"
        type="file"
        accept={REFERENCE_IMAGE_MIME_TYPES.join(",")}
        disabled={isBusy}
        onChange={handleFileChange}
        className="sr-only"
      />
      {isBusy ? (
        <p className="mt-2.5 text-xs font-semibold text-[var(--aria-ink-muted)]">
          处理中不可修改参考图
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="mt-3 rounded-lg border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
          {error}
        </p>
      ) : null}
    </section>
  );
}
