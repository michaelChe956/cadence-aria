import type { ChangeEvent } from "react";
import { BTN_SECONDARY_CLASS, GATE_INPUT_CLASS } from "../gate-visual-tokens";

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
  // F-50 视觉 v2 §2：反馈输入行——input 中性灰底 slate 描边（dark: bg-slate-800
  // border-slate-700），提交=次操作描边钮（旧琥珀描边形态退役：琥珀不再上按钮）。

  return (
    <div data-testid="gate-feedback-editor" className="flex flex-wrap items-center gap-2">
      {multiline ? (
        <textarea
          aria-label="门禁反馈"
          className={GATE_INPUT_CLASS}
          value={value}
          onChange={handleChange}
          placeholder="请输入反馈内容"
        />
      ) : (
        <input
          aria-label="门禁反馈"
          className={GATE_INPUT_CLASS}
          value={value}
          onChange={handleChange}
          placeholder="请输入反馈内容"
        />
      )}
      <button
        type="button"
        disabled={!value.trim()}
        onClick={() => onSubmit(value.trim())}
        className={`${BTN_SECONDARY_CLASS} disabled:cursor-not-allowed disabled:opacity-60`}
      >
        提交反馈
      </button>
    </div>
  );
}
