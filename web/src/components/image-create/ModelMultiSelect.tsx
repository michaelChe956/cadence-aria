import { ChevronDown, Plus } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";

export interface ModelMultiSelectProps {
  /** 候选池（网关返回 ∪ 已配置）。 */
  candidates: readonly string[];
  /** 已勾选（= 要保存的清单）。 */
  selected: readonly string[];
  /** 候选里但网关模型列表没有的（打「网关未列出」标）。 */
  notListed?: readonly string[];
  disabled?: boolean;
  onChange: (next: string[]) => void;
}

/** 触发器外观与 CustomSelect 同款：边框、圆角、内边距、focus ring 一致。 */
const triggerClassName =
  "flex min-h-11 w-full items-center justify-between gap-3 rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3.5 py-2.5 text-left text-base font-medium text-[var(--aria-ink)] shadow-[inset_0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)] disabled:opacity-60 sm:text-sm";

/** 候选池为空时的占位文案。 */
const EMPTY_HINT =
  "暂无模型，点「测试连通」可拉取网关支持的模型，或手动添加";

/** 按候选池顺序排列已选 id：池内的按池序，池外的追加在后。 */
function orderByCandidates(
  pool: readonly string[],
  ids: readonly string[],
): string[] {
  const chosen = new Set(ids);
  const ordered = pool.filter((model) => chosen.has(model));
  for (const id of ids) {
    if (!pool.includes(id)) {
      ordered.push(id);
    }
  }
  return ordered;
}

/**
 * 模型清单的复选下拉框：收起时显示已选数量，展开后逐项勾选，
 * 底部可添加网关没有的自定义模型 id。
 */
export function ModelMultiSelect({
  candidates,
  selected,
  notListed = [],
  disabled = false,
  onChange,
}: ModelMultiSelectProps) {
  const [expanded, setExpanded] = useState(false);
  const [customId, setCustomId] = useState("");
  const id = useId();
  const panelId = `${id}-panel`;
  const customInputId = `${id}-custom`;
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const selectedSet = new Set(selected);
  const notListedSet = new Set(notListed);

  useEffect(() => {
    if (!expanded) {
      return;
    }
    function handleMouseDown(event: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setExpanded(false);
      }
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        setExpanded(false);
        triggerRef.current?.focus();
      }
    }
    document.addEventListener("mousedown", handleMouseDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleMouseDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [expanded]);

  function toggle(model: string) {
    if (selectedSet.has(model)) {
      onChange(selected.filter((item) => item !== model));
      return;
    }
    onChange(orderByCandidates(candidates, [...selected, model]));
  }

  /** 添加自定义模型：并入已勾选集合（候选池由上层并入），空输入不生效。 */
  function handleAddCustom() {
    const model = customId.trim();
    if (model === "") {
      return;
    }
    if (!selectedSet.has(model)) {
      onChange(orderByCandidates(candidates, [...selected, model]));
    }
    setCustomId("");
  }

  return (
    <div ref={rootRef} className="relative">
      <button
        ref={triggerRef}
        type="button"
        aria-expanded={expanded}
        aria-controls={panelId}
        disabled={disabled}
        onClick={() => setExpanded((value) => !value)}
        className={triggerClassName}
      >
        <span className="min-w-0 flex-1 truncate">已选 {selected.length} 项</span>
        <ChevronDown
          aria-hidden="true"
          className={`h-4 w-4 shrink-0 text-[var(--aria-ink-muted)] transition-transform duration-200 motion-reduce:transition-none ${
            expanded ? "rotate-180 text-[var(--aria-primary)]" : ""
          }`}
        />
      </button>

      {expanded ? (
        <div
          id={panelId}
          className="mt-2 overflow-hidden rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-[0_4px_12px_rgba(0,0,0,0.08),0_12px_32px_rgba(0,0,0,0.12)]"
        >
          {candidates.length === 0 ? (
            <p className="px-3.5 py-2.5 text-xs text-[var(--aria-ink-muted)]">
              {EMPTY_HINT}
            </p>
          ) : (
            <ul className="max-h-60 overflow-y-auto p-1.5">
              {candidates.map((model) => (
                <li key={model}>
                  <label className="flex cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm text-[var(--aria-ink)] transition-colors duration-150 hover:bg-[var(--aria-primary-soft)]">
                    <input
                      type="checkbox"
                      checked={selectedSet.has(model)}
                      disabled={disabled}
                      onChange={() => toggle(model)}
                      aria-label={model}
                      className="h-4 w-4 shrink-0 cursor-pointer accent-[var(--aria-primary)] disabled:cursor-not-allowed"
                    />
                    <span className="min-w-0 flex-1 truncate font-mono">
                      {model}
                    </span>
                    {notListedSet.has(model) ? (
                      <span className="shrink-0 rounded-full border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-0.5 text-xs text-[var(--aria-ink-muted)]">
                        网关未列出
                      </span>
                    ) : null}
                  </label>
                </li>
              ))}
            </ul>
          )}

          <div className="flex items-center gap-2 border-t border-[var(--aria-line)] px-2.5 py-2">
            <label htmlFor={customInputId} className="sr-only">
              添加自定义模型
            </label>
            <input
              id={customInputId}
              value={customId}
              disabled={disabled}
              placeholder="添加自定义模型"
              spellCheck={false}
              onChange={(event) => setCustomId(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  handleAddCustom();
                }
              }}
              className="min-h-9 min-w-0 flex-1 rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2.5 py-1.5 text-sm text-[var(--aria-ink)] transition-colors duration-200 hover:border-[var(--aria-primary)] focus-visible:border-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
            />
            <button
              type="button"
              onClick={handleAddCustom}
              disabled={disabled || customId.trim() === ""}
              className="inline-flex shrink-0 cursor-pointer items-center gap-1 rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2.5 py-1.5 text-xs font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
            >
              <Plus aria-hidden="true" className="h-3.5 w-3.5" />
              添加
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
