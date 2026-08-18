import { CheckCircle2 } from "lucide-react";

interface FinalizeButtonProps {
  lineLabel: string;
  disabled?: boolean;
  disabledReason?: string;
  loading?: boolean;
  onClick: () => void;
}

/** 产物线定稿操作；前置未满足时保留可见的原因提示。 */
export function FinalizeButton({
  lineLabel,
  disabled = false,
  disabledReason,
  loading = false,
  onClick,
}: FinalizeButtonProps) {
  return (
    <button
      type="button"
      aria-label={`定稿${lineLabel}`}
      title={disabledReason}
      disabled={disabled || loading}
      onClick={onClick}
      className="inline-flex h-8 items-center gap-1.5 rounded-md bg-[var(--aria-primary)] px-2.5 text-xs font-semibold text-white hover:brightness-95 disabled:cursor-not-allowed disabled:opacity-50"
    >
      <CheckCircle2 aria-hidden="true" className="h-3.5 w-3.5" />
      {loading ? "定稿中…" : "定稿"}
    </button>
  );
}
