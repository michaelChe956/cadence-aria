import { Check, ChevronDown } from "lucide-react";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { createPortal } from "react-dom";

export type CustomSelectOption =
  | string
  | {
      value: string;
      label: string;
      disabled?: boolean;
      reason?: string | null;
    };

export interface CustomSelectProps {
  label?: string;
  value: string;
  /** 触发按钮上展示的文案，缺省时展示 value 本身。 */
  displayValue?: string;
  options: readonly CustomSelectOption[];
  disabled?: boolean;
  onChange: (value: string) => void;
  "aria-label"?: string;
}

const triggerClassName =
  "flex min-h-11 w-full items-center justify-between gap-3 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3.5 py-2.5 text-left text-base font-medium text-[var(--aria-ink)] shadow-[inset_0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)] disabled:opacity-60 sm:text-sm";


// portal 面板与 trigger 的垂直间距（对应原 mt-2）与高度上限（对应 max-h-60）。
const LIST_GAP_PX = 8;
const LIST_MAX_HEIGHT_PX = 240;

type ListPosition = {
  left: number;
  width: number;
  top?: number;
  bottom?: number;
  placement: "below" | "above";
};

export function CustomSelect({
  label,
  value,
  displayValue,
  options,
  disabled = false,
  onChange,
  "aria-label": ariaLabel,
}: CustomSelectProps) {
  const [expanded, setExpanded] = useState(false);
  const [renderList, setRenderList] = useState(false);
  const [listVisible, setListVisible] = useState(false);
  const [listPosition, setListPosition] = useState<ListPosition | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
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

  // 面板经 portal 挂 body 后用 fixed 定位：对齐 trigger 左缘与宽度，下方放不下时向上
  // 展开；弹窗内容区滚动不冒泡，scroll 必须走 capture 阶段才能跟随重算坐标。
  useLayoutEffect(() => {
    if (!renderList) {
      return;
    }
    function updatePosition() {
      const trigger = triggerRef.current;
      if (!trigger) {
        return;
      }
      const rect = trigger.getBoundingClientRect();
      const listHeight = Math.min(
        listRef.current?.offsetHeight ?? LIST_MAX_HEIGHT_PX,
        LIST_MAX_HEIGHT_PX,
      );
      const fitsBelow =
        window.innerHeight - rect.bottom - LIST_GAP_PX >= listHeight;
      const fitsAbove = rect.top - LIST_GAP_PX >= listHeight;
      const openAbove = !fitsBelow && fitsAbove;
      setListPosition({
        left: rect.left,
        width: rect.width,
        top: openAbove ? undefined : rect.bottom + LIST_GAP_PX,
        bottom: openAbove
          ? window.innerHeight - rect.top + LIST_GAP_PX
          : undefined,
        placement: openAbove ? "above" : "below",
      });
    }

    updatePosition();
    window.addEventListener("scroll", updatePosition, { capture: true });
    window.addEventListener("resize", updatePosition);
    return () => {
      window.removeEventListener("scroll", updatePosition, { capture: true });
      window.removeEventListener("resize", updatePosition);
    };
  }, [renderList]);

  useEffect(() => {
    function handleDocumentMouseDown(event: MouseEvent) {
      const target = event.target as Node | null;
      if (
        expanded &&
        target &&
        !rootRef.current?.contains(target) &&
        !listRef.current?.contains(target)
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
    option: CustomSelectOption,
  ) {
    event.preventDefault();
    const optionValue = typeof option === "string" ? option : option.value;
    const optionDisabled = typeof option === "string" ? false : option.disabled === true;
    if (optionDisabled) {
      return;
    }
    if (optionValue !== value) {
      onChange(optionValue);
    }
    closeList({ restoreFocus: true });
  }

  return (
    <div ref={rootRef}>
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
        <span className="min-w-0 flex-1 truncate">
          {displayValue ?? value}
        </span>
        <ChevronDown
          aria-hidden="true"
          className={`h-4 w-4 shrink-0 text-[var(--aria-ink-muted)] transition-transform duration-200 motion-reduce:transition-none ${
            expanded ? "rotate-180 text-[var(--aria-primary)]" : ""
          }`}
        />
      </button>

      {renderList
        ? createPortal(
            <div
              ref={listRef}
              id={listboxId}
              role="listbox"
              aria-label={accessibleLabel}
              aria-hidden={!expanded}
              style={{
                left: listPosition?.left,
                width: listPosition?.width,
                top: listPosition?.top,
                bottom: listPosition?.bottom,
              }}
              className={`fixed z-50 max-h-60 overflow-y-auto rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-1 shadow-[0_4px_12px_rgba(0,0,0,0.08),0_12px_32px_rgba(0,0,0,0.12)] transition-[opacity,transform] duration-150 motion-reduce:transition-none ${
                listPosition?.placement === "above"
                  ? "origin-bottom"
                  : "origin-top"
              } ${
                listVisible
                  ? "scale-100 opacity-100"
                  : "pointer-events-none scale-[0.98] opacity-0"
              }`}
            >
              {options.map((option) => {
                const optionValue = typeof option === "string" ? option : option.value;
                const optionLabel = typeof option === "string" ? option : option.label;
                const optionDisabled =
                  typeof option === "string" ? false : option.disabled === true;
                const optionReason = typeof option === "string" ? null : option.reason;
                const selected = optionValue === value;
                return (
                  <button
                    key={optionValue}
                    type="button"
                    role="option"
                    aria-selected={selected}
                    disabled={optionDisabled}
                    onClick={(event) => handleOptionClick(event, option)}
                    className={`flex min-h-11 w-full items-center gap-2 rounded-lg border-l-2 px-3.5 py-2 text-left text-base font-medium transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--aria-primary)] sm:text-sm ${
                      selected
                        ? "border-l-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-ink)]"
                        : "border-l-transparent text-[var(--aria-ink)]"
                    } ${
                      optionDisabled
                        ? "cursor-not-allowed opacity-50"
                        : "cursor-pointer hover:bg-[var(--aria-panel-muted)]"
                    }`}
                  >
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium">
                        {optionLabel}
                      </span>
                      {optionReason ? (
                        <span className="mt-0.5 block text-xs leading-snug text-[var(--aria-ink-muted)]">
                          {optionReason}
                        </span>
                      ) : null}
                    </span>
                    {selected ? (
                      <Check
                        aria-hidden="true"
                        className="h-4 w-4 shrink-0 text-[var(--aria-primary)]"
                      />
                    ) : null}
                  </button>
                );
              })}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
