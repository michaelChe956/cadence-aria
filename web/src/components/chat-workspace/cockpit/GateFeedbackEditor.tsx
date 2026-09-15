import type { ChangeEvent } from "react";

export function GateFeedbackEditor({
  multiline = false,
  value,
  onChange,
  onSubmit,
}: {
  multiline?: boolean;
  value: string;
  onChange(value: string): void;
  onSubmit(value: string): void;
}) {
  const handleChange = (event: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    onChange(event.target.value);
  };
  const className = "min-h-11 min-w-0 flex-1 rounded-md border border-[var(--aria-line-strong)] bg-white px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]";

  return (
    <div data-testid="gate-feedback-editor" className="flex flex-wrap items-center gap-2">
      {multiline ? (
        <textarea
          aria-label="门禁反馈"
          value={value}
          onChange={handleChange}
          placeholder="请输入反馈内容"
          className={className}
        />
      ) : (
        <input
          aria-label="门禁反馈"
          value={value}
          onChange={handleChange}
          placeholder="请输入反馈内容"
          className={className}
        />
      )}
      <button
        type="button"
        disabled={!value.trim()}
        onClick={() => onSubmit(value.trim())}
        className="inline-flex min-h-11 items-center gap-1 rounded-md border border-amber-200 bg-white px-3 text-xs font-semibold text-amber-700 hover:bg-amber-50 disabled:cursor-not-allowed disabled:opacity-60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        提交反馈
      </button>
    </div>
  );
}
