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
      className="rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-sm"
    >
      <h2 id="image-create-reference-heading" className="text-base font-semibold">
        参考图
      </h2>
      <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
        支持 PNG、JPEG、WebP，单张不超过 10MB。
      </p>
      {referenceImage && previewUrl ? (
        <div className="mt-3 overflow-hidden rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)]">
          <img
            src={previewUrl}
            alt="参考图预览"
            className="max-h-48 w-full object-contain"
          />
          <div className="flex items-center justify-between gap-3 border-t border-[var(--aria-line)] px-3 py-2">
            <span className="min-w-0 truncate text-xs text-[var(--aria-ink-muted)]">
              {referenceImage.name}
            </span>
            <button
              type="button"
              onClick={removeReference}
              disabled={isBusy}
              title={isBusy ? "处理中不可修改参考图" : undefined}
              className="shrink-0 rounded-md border border-[var(--aria-line)] bg-white px-2 py-1 text-xs font-semibold text-[var(--aria-danger)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              移除参考图
            </button>
          </div>
        </div>
      ) : (
        <label
          htmlFor={isBusy ? undefined : inputId}
          aria-disabled={isBusy}
          title={isBusy ? "处理中不可修改参考图" : undefined}
          className={`mt-3 flex flex-col items-center rounded-lg border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-6 text-center ${
            isBusy
              ? "cursor-not-allowed opacity-60"
              : "cursor-pointer hover:bg-white"
          }`}
        >
          <span className="text-sm font-semibold">选择一张参考图</span>
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
        <p className="mt-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
          处理中不可修改参考图
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="mt-2 text-sm font-semibold text-[var(--aria-danger)]">
          {error}
        </p>
      ) : null}
    </section>
  );
}
