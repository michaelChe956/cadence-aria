import { useMemo, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import type { RoleInstance } from "../../api/groupChat";

interface MentionInputProps {
  roles: RoleInstance[];
  onSubmit: (text: string, mentions: string[]) => void;
  disabled?: boolean;
}

/** 群聊输入框：展示角色实例名，并向服务端提交对应的实例 ID。 */
export function MentionInput({
  roles,
  onSubmit,
  disabled = false,
}: MentionInputProps) {
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const [value, setValue] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const mention = activeMention(value);
  const matches = useMemo(() => {
    if (mention === null) {
      return [];
    }
    const keyword = mention.keyword.toLocaleLowerCase();
    return roles.filter((role) =>
      [role.display_name, role.role_key]
        .some((value) => value.toLocaleLowerCase().includes(keyword)),
    );
  }, [mention, roles]);

  function selectRole(role: RoleInstance) {
    if (!mention) {
      return;
    }
    setValue(
      `${value.slice(0, mention.start)}${mention.leading}@${role.display_name}${value.slice(mention.end)}`,
    );
    setActiveIndex(0);
    inputRef.current?.focus();
  }

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const text = value.trim();
    if (!text || disabled) {
      return;
    }
    onSubmit(
      text,
      roles
        .filter((role) => text.includes(`@${role.display_name}`))
        .map((role) => role.id),
    );
    setValue("");
    setActiveIndex(0);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (matches.length === 0) {
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActiveIndex((current) => (current + 1) % matches.length);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex((current) => (current - 1 + matches.length) % matches.length);
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      selectRole(matches[activeIndex] ?? matches[0]);
    }
  }

  return (
    <form
      data-testid="group-chat-mention-input"
      onSubmit={handleSubmit}
      className="border-t border-[var(--aria-line)] bg-[var(--aria-panel)] p-3"
    >
      <div className="relative">
        <textarea
          ref={inputRef}
          aria-label="群聊消息"
          value={value}
          disabled={disabled}
          rows={3}
          placeholder="输入消息；使用 @ 提及角色"
          onChange={(event) => {
            setValue(event.target.value);
            setActiveIndex(0);
          }}
          onKeyDown={handleKeyDown}
          className="min-h-20 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 pr-20 text-sm text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)] disabled:bg-[var(--aria-panel-muted)]"
        />
        {mention && matches.length > 0 ? (
          <div
            role="listbox"
            aria-label="提及角色"
            className="absolute bottom-full left-0 z-10 mb-2 max-h-52 w-full overflow-auto rounded-md border border-[var(--aria-line)] bg-white p-1 shadow-lg"
          >
            {matches.map((role, index) => (
              <button
                key={role.id}
                role="option"
                aria-selected={index === activeIndex}
                type="button"
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => selectRole(role)}
                className={[
                  "flex w-full items-center justify-between gap-3 rounded px-2.5 py-2 text-left text-sm",
                  index === activeIndex
                    ? "bg-[var(--aria-primary-soft)] text-[var(--aria-ink)]"
                    : "hover:bg-[var(--aria-panel-muted)]",
                ].join(" ")}
              >
                <span className="font-medium">{role.display_name}</span>
                <span
                  aria-hidden="true"
                  className="shrink-0 text-xs text-[var(--aria-ink-muted)]"
                >
                  {role.role_key}
                </span>
              </button>
            ))}
          </div>
        ) : null}
      </div>
      <div className="mt-2 flex justify-end">
        <button
          type="submit"
          disabled={disabled || value.trim().length === 0}
          className="btn-primary h-9 disabled:opacity-50"
        >
          发送
        </button>
      </div>
    </form>
  );
}

function activeMention(value: string): {
  start: number;
  end: number;
  leading: string;
  keyword: string;
} | null {
  const match = /(^|\s)@([^\s@]*)$/u.exec(value);
  if (!match || match.index === undefined) {
    return null;
  }
  return {
    start: match.index,
    end: value.length,
    leading: match[1] ?? "",
    keyword: match[2] ?? "",
  };
}
