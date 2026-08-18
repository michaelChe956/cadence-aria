import { useEffect, useMemo, useState, type FormEvent } from "react";
import {
  addGroupChatRole,
  type GroupChatProviderName,
  type GroupChatRoleKey,
  type GroupChatSession,
} from "../../api/groupChat";
import { getProviderOptions } from "../../state/provider-options";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";

export const GROUP_CHAT_ROLE_LABELS: Record<GroupChatRoleKey, string> = {
  author: "需求作者",
  frontend_design: "前端设计",
  backend_design: "后端设计",
  reviewer: "审核员",
  researcher: "资料研究员",
};

const ROLE_KEYS: GroupChatRoleKey[] = [
  "author",
  "frontend_design",
  "backend_design",
  "reviewer",
  "researcher",
];

interface AddRoleDialogProps {
  sessionId: string;
  onClose: () => void;
  onAdded: (session: GroupChatSession) => void;
}

/** 添加角色实例；同一角色允许通过不同实例重复加入会话。 */
export function AddRoleDialog({
  sessionId,
  onClose,
  onAdded,
}: AddRoleDialogProps) {
  const snapshot = useProviderAvailabilityStore((state) => state.snapshot);
  const providerLoadStatus = useProviderAvailabilityStore(
    (state) => state.loadStatus,
  );
  const loadProviders = useProviderAvailabilityStore((state) => state.load);
  const providerOptions = useMemo(
    () => getProviderOptions(snapshot).filter((option) => option.visible),
    [snapshot],
  );
  const [roleKey, setRoleKey] = useState<GroupChatRoleKey>("author");
  const [provider, setProvider] = useState<GroupChatProviderName>("claude_code");
  const [displayName, setDisplayName] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (snapshot === null && providerLoadStatus === "idle") {
      void loadProviders();
    }
  }, [loadProviders, providerLoadStatus, snapshot]);

  useEffect(() => {
    const selected = providerOptions.find((option) => option.value === provider);
    const available = providerOptions.find((option) => !option.disabled);
    if (!selected || (selected.disabled && available)) {
      setProvider(available?.value ?? providerOptions[0]?.value ?? "claude_code");
    }
  }, [provider, providerOptions]);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submitting || !canSubmit) {
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const session = await addGroupChatRole(sessionId, {
        role_key: roleKey,
        provider,
        ...(displayName.trim() ? { display_name: displayName.trim() } : {}),
      });
      onAdded(session);
    } catch (cause: unknown) {
      setError(cause instanceof Error ? cause.message : "添加角色失败");
      setSubmitting(false);
    }
  }

  const selectedProvider = providerOptions.find(
    (option) => option.value === provider,
  );
  const canSubmit = Boolean(
    selectedProvider && !selectedProvider.disabled && !submitting,
  );

  return (
    <div
      className="fixed inset-0 z-30 flex items-center justify-center bg-black/30 p-4"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !submitting) {
          onClose();
        }
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="add-role-dialog-title"
        className="w-full max-w-md rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="flex items-center justify-between gap-3">
          <h2 id="add-role-dialog-title" className="text-base font-semibold">
            添加角色
          </h2>
          <button
            type="button"
            aria-label="关闭"
            onClick={onClose}
            disabled={submitting}
            className="rounded px-2 py-1 text-sm text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
          >
            关闭
          </button>
        </div>

        <form className="mt-4 space-y-3" onSubmit={handleSubmit}>
          <label className="block text-sm">
            <span className="mb-1 block text-[var(--aria-ink-muted)]">角色</span>
            <select
              aria-label="角色"
              value={roleKey}
              onChange={(event) => setRoleKey(event.target.value as GroupChatRoleKey)}
              disabled={submitting}
              className="w-full rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm"
            >
              {ROLE_KEYS.map((key) => (
                <option key={key} value={key}>
                  {GROUP_CHAT_ROLE_LABELS[key]}
                </option>
              ))}
            </select>
          </label>

          <label className="block text-sm">
            <span className="mb-1 block text-[var(--aria-ink-muted)]">Provider</span>
            <select
              aria-label="Provider"
              value={provider}
              onChange={(event) =>
                setProvider(event.target.value as GroupChatProviderName)
              }
              disabled={submitting || providerOptions.length === 0}
              className="w-full rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm disabled:bg-[var(--aria-panel-muted)]"
            >
              {providerOptions.map((option) => (
                <option key={option.value} value={option.value} disabled={option.disabled}>
                  {option.label}
                  {option.disabled && option.reason ? `（${option.reason}）` : ""}
                </option>
              ))}
            </select>
            {providerLoadStatus === "loading" ? (
              <span className="mt-1 block text-xs text-[var(--aria-ink-muted)]">
                正在读取 Provider 状态…
              </span>
            ) : null}
          </label>

          <label className="block text-sm">
            <span className="mb-1 block text-[var(--aria-ink-muted)]">显示名（可选）</span>
            <input
              aria-label="显示名（可选）"
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
              disabled={submitting}
              placeholder={`默认：${GROUP_CHAT_ROLE_LABELS[roleKey]}`}
              className="w-full rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm"
            />
          </label>

          {error ? (
            <p role="alert" className="rounded-md bg-red-50 px-2 py-1.5 text-sm text-red-700">
              {error}
            </p>
          ) : null}

          <div className="flex justify-end gap-2 pt-1">
            <button
              type="button"
              onClick={onClose}
              disabled={submitting}
              className="rounded-md border border-[var(--aria-line)] px-3 py-1.5 text-sm hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
            >
              取消
            </button>
            <button
              type="submit"
              disabled={!canSubmit}
              className="btn-primary rounded-md px-3 py-1.5 text-sm disabled:opacity-50"
            >
              {submitting ? "添加中…" : "添加"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
