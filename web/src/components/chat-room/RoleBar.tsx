import { useState } from "react";
import type { GroupChatSession, RoleInstance } from "../../api/groupChat";
import { getProviderOption } from "../../state/provider-options";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import {
  AddRoleDialog,
  GROUP_CHAT_ROLE_LABELS,
} from "./AddRoleDialog";

interface RoleBarProps {
  sessionId: string;
  roles: RoleInstance[];
  onSessionUpdated: (session: GroupChatSession) => void;
  disabled?: boolean;
}

/** 群聊角色栏：展示实例身份及权限，并提供添加角色入口。 */
export function RoleBar({
  sessionId,
  roles,
  onSessionUpdated,
  disabled = false,
}: RoleBarProps) {
  const snapshot = useProviderAvailabilityStore((state) => state.snapshot);
  const [dialogOpen, setDialogOpen] = useState(false);

  return (
    <section
      aria-label="群聊角色"
      className="shrink-0 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2"
    >
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-sm font-semibold">角色</h2>
        <button
          type="button"
          aria-label="添加角色"
          disabled={disabled}
          onClick={() => setDialogOpen(true)}
          className="rounded-md border border-[var(--aria-line)] px-2.5 py-1 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] disabled:cursor-not-allowed disabled:opacity-50"
        >
          添加角色
        </button>
      </div>
      <div className="mt-2 flex min-w-0 gap-2 overflow-x-auto pb-0.5">
        {roles.map((role) => {
          const provider = getProviderOption(snapshot, role.provider);
          const readOnly = role.role_key === "reviewer" || role.role_key === "researcher";
          return (
            <article
              key={role.id}
              data-testid={`role-card-${role.role_key}-${role.id}`}
              className="min-w-44 rounded-md border border-[var(--aria-line)] bg-white px-2.5 py-2"
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="truncate text-sm font-semibold" title={role.display_name}>
                    {role.display_name}
                  </p>
                  <p className="text-xs text-[var(--aria-ink-muted)]">
                    {GROUP_CHAT_ROLE_LABELS[role.role_key]}
                  </p>
                </div>
                {readOnly ? (
                  <span className="shrink-0 rounded-full bg-[var(--aria-panel-muted)] px-1.5 py-0.5 text-[10px] font-semibold text-[var(--aria-ink-muted)]">
                    只读
                  </span>
                ) : null}
              </div>
              <p className="mt-1 truncate text-xs text-[var(--aria-ink-muted)]">
                Provider：{provider.label}
              </p>
              {readOnly ? (
                <p className="mt-1 text-[11px] text-[var(--aria-ink-muted)]">
                  只读角色不可写入产物
                </p>
              ) : null}
            </article>
          );
        })}
        {roles.length === 0 ? (
          <p className="py-2 text-xs text-[var(--aria-ink-muted)]">暂无角色</p>
        ) : null}
      </div>
      {dialogOpen ? (
        <AddRoleDialog
          sessionId={sessionId}
          onClose={() => setDialogOpen(false)}
          onAdded={(session) => {
            setDialogOpen(false);
            onSessionUpdated(session);
          }}
        />
      ) : null}
    </section>
  );
}
