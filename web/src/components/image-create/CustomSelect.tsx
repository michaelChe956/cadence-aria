import { Check, ChevronDown } from "lucide-react";
import {
  useEffect,
  useId,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";

export interface CustomSelectProps {
  label?: string;
  value: string;
  options: readonly string[];
  disabled?: boolean;
  onChange: (value: string) => void;
  "aria-label"?: string;
}

const triggerClassName =
  "flex w-full items-center justify-between gap-3 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3.5 py-2.5 text-left text-sm font-medium text-[var(--aria-ink)] shadow-[inset_0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)] disabled:opacity-60";

export function CustomSelect({
  label,
  value,
  options,
  disabled = false,
  onChange,
  "aria-label": ariaLabel,
}: CustomSelectProps) {
  const [expanded, setExpanded] = useState(false);
  const [renderList, setRenderList] = useState(false);
  const [listVisible, setListVisible] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const closeTimerRef = useRef<number | null>(null);
  const animationFrameRef = useRef<number | null>(null);
  const id = useId();
  const labelId = `${id}-label`;
  const listboxId = `${id}-listbox`;
  const accessibleLabel = ariaLabel ?? label;

  function clearCloseTimer() {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  }

  function openList() {
    if (disabled) {
      return;
    }
    clearCloseTimer();
    setRenderList(true);
    setExpanded(true);
  }

  function closeList({ restoreFocus = false } = {}) {
    setExpanded(false);
    setListVisible(false);
    clearCloseTimer();
    closeTimerRef.current = window.setTimeout(() => {
      setRenderList(false);
      closeTimerRef.current = null;
    }, 150);
    if (restoreFocus) {
      triggerRef.current?.focus();
    }
  }

  useEffect(() => {
    if (!expanded || !renderList) {
      return;
    }
    animationFrameRef.current = window.requestAnimationFrame(() => {
      setListVisible(true);
      animationFrameRef.current = null;
    });
    return () => {
      if (animationFrameRef.current !== null) {
        window.cancelAnimationFrame(animationFrameRef.current);
        animationFrameRef.current = null;
      }
    };
  }, [expanded, renderList]);

  useEffect(() => {
    function handleDocumentMouseDown(event: MouseEvent) {
      if (
        expanded &&
        rootRef.current &&
        !rootRef.current.contains(event.target as Node)
      ) {
        closeList();
      }
    }

    function handleDocumentKeyDown(event: KeyboardEvent) {
      if (expanded && event.key === "Escape") {
        event.preventDefault();
        closeList({ restoreFocus: true });
      }
    }

    document.addEventListener("mousedown", handleDocumentMouseDown);
    document.addEventListener("keydown", handleDocumentKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleDocumentMouseDown);
      document.removeEventListener("keydown", handleDocumentKeyDown);
    };
  }, [expanded]);

  useEffect(() => {
    if (disabled && expanded) {
      closeList();
    }
  }, [disabled, expanded]);

  useEffect(
    () => () => {
      clearCloseTimer();
      if (animationFrameRef.current !== null) {
        window.cancelAnimationFrame(animationFrameRef.current);
      }
    },
  );

  function handleOptionClick(
    event: ReactMouseEvent<HTMLButtonElement>,
    option: string,
  ) {
    event.preventDefault();
    if (option !== value) {
      onChange(option);
    }
    closeList({ restoreFocus: true });
  }

  return (
    <div ref={rootRef} className="relative">
      {label ? (
        <span
          id={labelId}
          className="mb-1.5 block text-sm font-semibold text-[var(--aria-ink)]"
        >
          {label}
        </span>
      ) : null}
      <button
        ref={triggerRef}
        type="button"
        role="combobox"
        aria-label={accessibleLabel}
        aria-labelledby={!ariaLabel && label ? labelId : undefined}
        aria-controls={listboxId}
        aria-expanded={expanded}
        aria-haspopup="listbox"
        disabled={disabled}
        onClick={() => (expanded ? closeList() : openList())}
        className={triggerClassName}
      >
        <span className="min-w-0 flex-1 truncate">{value}</span>
        <ChevronDown
          aria-hidden="true"
          className={`h-4 w-4 shrink-0 text-[var(--aria-ink-muted)] transition-transform duration-200 motion-reduce:transition-none ${
            expanded ? "rotate-180 text-[var(--aria-primary)]" : ""
          }`}
        />
      </button>

      {renderList ? (
        <div
          id={listboxId}
          role="listbox"
          aria-label={accessibleLabel}
          aria-hidden={!expanded}
          className={`absolute left-0 right-0 top-full z-50 mt-2 max-h-60 origin-top overflow-y-auto rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-1.5 shadow-[0_4px_12px_rgba(0,0,0,0.08),0_12px_32px_rgba(0,0,0,0.12)] transition-[opacity,transform] duration-150 motion-reduce:transition-none ${
            listVisible
              ? "scale-100 opacity-100"
              : "pointer-events-none scale-[0.98] opacity-0"
          }`}
        >
          {options.map((option) => {
            const selected = option === value;
            return (
              <button
                key={option}
                type="button"
                role="option"
                aria-selected={selected}
                onClick={(event) => handleOptionClick(event, option)}
                className={`flex w-full cursor-pointer items-center gap-2 rounded-lg border-l-2 px-3.5 py-2.5 text-left text-sm font-medium transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--aria-primary)] ${
                  selected
                    ? "border-l-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
                    : "border-l-transparent text-[var(--aria-ink)] hover:bg-[var(--aria-primary-soft)] hover:text-[var(--aria-primary)]"
                }`}
              >
                <span className="min-w-0 flex-1 truncate">{option}</span>
                {selected ? (
                  <Check
                    aria-hidden="true"
                    className="h-4 w-4 shrink-0 text-[var(--aria-primary)]"
                  />
                ) : null}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
