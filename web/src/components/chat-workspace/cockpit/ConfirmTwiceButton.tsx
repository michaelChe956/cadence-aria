import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useState,
} from "react";
import {
  DANGEROUS_CONFIRM_TIMEOUT_MS,
  type ConfirmTwiceButtonHandle,
} from "../../../state/cockpit-operation-semantics";

export const ConfirmTwiceButton = forwardRef<ConfirmTwiceButtonHandle, {
  label: string;
  confirmLabel: string;
  onConfirm: () => void;
}>(function ConfirmTwiceButton({ label, confirmLabel, onConfirm }, ref) {
  const [armed, setArmed] = useState(false);
  const arm = useCallback(() => {
    if (armed) {
      setArmed(false);
      onConfirm();
      return;
    }
    setArmed(true);
  }, [armed, onConfirm]);

  useImperativeHandle(ref, () => ({ arm }), [arm]);

  useEffect(() => {
    if (!armed) {
      return;
    }
    const timer = window.setTimeout(() => setArmed(false), DANGEROUS_CONFIRM_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [armed]);

  return (
    <button
      data-testid="confirm-twice-button"
      type="button"
      onClick={arm}
      className="inline-flex min-h-11 items-center gap-1 rounded-md border border-red-200 bg-white px-3 text-xs font-semibold text-red-700 hover:bg-red-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
    >
      {armed ? confirmLabel : label}
    </button>
  );
});
