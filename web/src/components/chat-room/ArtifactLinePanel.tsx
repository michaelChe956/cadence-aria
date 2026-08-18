import { FilePenLine, Layers3 } from "lucide-react";
import { useMemo, useState } from "react";
import { finalizeGroupChat } from "../../api/groupChat";
import { ApiRequestError } from "../../api/client";
import type {
  ArtifactLine,
  ArtifactLineKind,
  DraftSlot,
  DraftSlotKey,
  GroupChatSession,
  RoleInstance,
} from "../../api/groupChat";
import { DraftPreview } from "./DraftPreview";
import { FinalizeButton } from "./FinalizeButton";

interface ArtifactLinePanelProps {
  sessionId: string;
  artifactLines: ArtifactLine[];
  roles: RoleInstance[];
  /** 页面负责组装并发送带 draft_slot 的群聊消息。 */
  onDraftSlot: (slotKey: DraftSlotKey) => void;
  /** HTTP 定稿完成后回写会话快照，避免等待下一次刷新。 */
  onSessionUpdated?: (session: GroupChatSession) => void;
  sessionActive?: boolean;
}

type PendingFinalize = {
  line: ArtifactLine;
  includedSlots: DraftSlotKey[];
  missingSlots: DraftSlot[];
};

const LINE_META: Record<ArtifactLineKind, { label: string; description: string }> = {
  issue_refinement: { label: "需求澄清", description: "问题背景与范围" },
  story_spec: { label: "故事规格", description: "用户故事与验收条件" },
  design_spec: { label: "设计规格", description: "设计方案" },
};

const SLOT_LABELS: Record<string, string> = {
  issue_full: "需求澄清",
  story_full: "故事规格",
  design_summary: "设计概要",
  design_frontend: "前端设计",
  design_backend: "后端设计",
};

const LINE_ORDER: ArtifactLineKind[] = [
  "issue_refinement",
  "story_spec",
  "design_spec",
];

const DESIGN_SLOT_ORDER = ["design_summary", "design_frontend", "design_backend"];

/** 群聊会话的三条产物线、草稿槽和定稿操作。 */
export function ArtifactLinePanel({
  sessionId,
  artifactLines,
  roles,
  onDraftSlot,
  onSessionUpdated,
  sessionActive = true,
}: ArtifactLinePanelProps) {
  const [pendingFinalize, setPendingFinalize] = useState<PendingFinalize | null>(null);
  const [finalizingKind, setFinalizingKind] = useState<ArtifactLineKind | null>(null);
  const [error, setError] = useState<string | null>(null);
  const linesByKind = useMemo(
    () => new Map(artifactLines.map((line) => [line.kind, line])),
    [artifactLines],
  );

  async function finalize(
    line: ArtifactLine,
    includedSlotsOverride?: DraftSlotKey[],
  ) {
    setError(null);
    setFinalizingKind(line.kind);
    try {
      const result = await finalizeGroupChat(sessionId, {
        line_kind: line.kind,
        ...(includedSlotsOverride ? { included_slots_override: includedSlotsOverride } : {}),
      });
      onSessionUpdated?.(result.session);
    } catch (cause: unknown) {
      setError(finalizeErrorMessage(cause));
    } finally {
      setFinalizingKind(null);
    }
  }

  function requestFinalize(line: ArtifactLine) {
    const missingSlots = line.drafts.filter((slot) => slot.current === null);
    if (line.kind === "design_spec" && missingSlots.length > 0) {
      setPendingFinalize({
        line,
        includedSlots: line.drafts
          .filter((slot) => slot.current !== null)
          .map((slot) => slot.slot_key),
        missingSlots,
      });
      return;
    }
    void finalize(line);
  }

  return (
    <aside
      data-testid="artifact-line-panel"
      className="border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-3 lg:w-80 lg:shrink-0 lg:border-l lg:border-t-0"
    >
      <div className="mb-3 flex items-center gap-2">
        <Layers3 aria-hidden="true" className="h-4 w-4 text-[var(--aria-primary)]" />
        <h2 className="text-sm font-semibold">产物线</h2>
      </div>
      {error ? (
        <div role="alert" className="mb-3 rounded-md border border-red-200 bg-red-50 px-2.5 py-2 text-xs text-red-700">
          {error}
        </div>
      ) : null}
      <div className="space-y-3">
        {LINE_ORDER.map((kind) => {
          const line = linesByKind.get(kind);
          if (!line) {
            return null;
          }
          return (
            <ArtifactLineCard
              key={kind}
              line={line}
              roles={roles}
              sessionActive={sessionActive}
              finalizing={finalizingKind === kind}
              onDraftSlot={onDraftSlot}
              onFinalize={() => requestFinalize(line)}
            />
          );
        })}
      </div>
      {pendingFinalize ? (
        <SkipMissingSlotsDialog
          missingSlots={pendingFinalize.missingSlots}
          busy={finalizingKind === pendingFinalize.line.kind}
          onCancel={() => setPendingFinalize(null)}
          onConfirm={() => {
            const pending = pendingFinalize;
            setPendingFinalize(null);
            void finalize(pending.line, pending.includedSlots);
          }}
        />
      ) : null}
    </aside>
  );
}

function ArtifactLineCard({
  line,
  roles,
  sessionActive,
  finalizing,
  onDraftSlot,
  onFinalize,
}: {
  line: ArtifactLine;
  roles: RoleInstance[];
  sessionActive: boolean;
  finalizing: boolean;
  onDraftSlot: (slotKey: DraftSlotKey) => void;
  onFinalize: () => void;
}) {
  const meta = LINE_META[line.kind];
  const status = artifactLineStatus(line, roles);
  const completedSlots = line.drafts.filter((slot) => slot.current !== null);
  const unavailableReason = finalizeUnavailableReason(
    line,
    completedSlots.length,
    sessionActive,
  );
  const slots = orderedSlots(line);

  return (
    <section
      data-testid={`artifact-line-${line.kind}`}
      className="rounded-lg border border-[var(--aria-line)] bg-white p-3 shadow-sm"
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold">{meta.label}</h3>
          <p className="mt-0.5 text-xs text-[var(--aria-ink-muted)]">{meta.description}</p>
        </div>
        <span className={`shrink-0 rounded-full px-2 py-0.5 text-xs font-medium ${status.tone}`}>
          {status.label}
        </span>
      </div>
      <div className="mt-3 space-y-2">
        {slots.map((slot) => (
          <DraftSlotCard
            key={slot.slot_key}
            slot={slot}
            roles={roles}
            sessionActive={sessionActive}
            onDraftSlot={onDraftSlot}
          />
        ))}
      </div>
      <div className="mt-3 flex justify-end">
        <FinalizeButton
          lineLabel={meta.label}
          disabled={unavailableReason !== null}
          disabledReason={unavailableReason ?? finalizeHint(line)}
          loading={finalizing}
          onClick={onFinalize}
        />
      </div>
    </section>
  );
}

function DraftSlotCard({
  slot,
  roles,
  sessionActive,
  onDraftSlot,
}: {
  slot: DraftSlot;
  roles: RoleInstance[];
  sessionActive: boolean;
  onDraftSlot: (slotKey: DraftSlotKey) => void;
}) {
  const label = slotLabel(slot.slot_key);
  const claimHolder = slot.claim
    ? roles.find((role) => role.id === slot.claim?.holder_role_id)?.display_name ?? "未知角色"
    : null;

  return (
    <div className="rounded-md bg-[var(--aria-panel-muted)] p-2">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-[var(--aria-ink)]">{label}</span>
        <button
          type="button"
          aria-label={`起草${label}`}
          disabled={!sessionActive}
          onClick={() => onDraftSlot(slot.slot_key)}
          className="inline-flex h-7 items-center gap-1 rounded border border-[var(--aria-line)] bg-white px-2 text-xs font-medium hover:bg-[var(--aria-panel)] disabled:cursor-not-allowed disabled:opacity-50"
        >
          <FilePenLine aria-hidden="true" className="h-3 w-3" />
          起草
        </button>
      </div>
      {claimHolder ? (
        <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">由{claimHolder}认领</p>
      ) : null}
      {slot.current ? <div className="mt-2"><DraftPreview slotLabel={label} draft={slot.current} /></div> : null}
    </div>
  );
}

function SkipMissingSlotsDialog({
  missingSlots,
  busy,
  onCancel,
  onConfirm,
}: {
  missingSlots: DraftSlot[];
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <section
        role="dialog"
        aria-modal="true"
        aria-label="确认跳过缺失草稿槽"
        className="w-full max-w-sm rounded-lg border border-[var(--aria-line)] bg-white p-4 shadow-xl"
      >
        <h3 className="text-sm font-semibold">确认跳过缺失草稿槽</h3>
        <p className="mt-2 text-sm text-[var(--aria-ink-muted)]">
          以下草稿槽尚未完成。确认后仅使用已有草稿槽定稿：
        </p>
        <ul className="mt-2 list-inside list-disc text-sm">
          {missingSlots.map((slot) => <li key={slot.slot_key}>{slotLabel(slot.slot_key)}</li>)}
        </ul>
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={onCancel}
            className="h-8 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-medium hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
          >
            取消
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={onConfirm}
            className="h-8 rounded-md bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:opacity-50"
          >
            {busy ? "定稿中…" : "确认跳过并定稿"}
          </button>
        </div>
      </section>
    </div>
  );
}

function artifactLineStatus(line: ArtifactLine, roles: RoleInstance[]) {
  if (line.finalized_versions.length > 0) {
    return { label: `已定稿 v${line.finalized_versions.length}`, tone: "bg-emerald-100 text-emerald-700" };
  }
  const claim = line.drafts.find((slot) => slot.claim)?.claim;
  if (claim) {
    const holder = roles.find((role) => role.id === claim.holder_role_id)?.display_name ?? "未知角色";
    return { label: `起草中（${holder}）`, tone: "bg-blue-100 text-blue-700" };
  }
  const completedSlots = line.drafts.filter((slot) => slot.current !== null).length;
  if (completedSlots === 0) {
    return { label: "未开始", tone: "bg-slate-100 text-slate-600" };
  }
  if (completedSlots === line.drafts.length) {
    return { label: "可定稿", tone: "bg-amber-100 text-amber-700" };
  }
  return { label: "待审", tone: "bg-violet-100 text-violet-700" };
}

function finalizeUnavailableReason(
  line: ArtifactLine,
  completedSlots: number,
  sessionActive: boolean,
): string | null {
  if (!sessionActive) {
    return "当前会话不可定稿";
  }
  if (completedSlots === 0) {
    return "暂无可定稿草稿";
  }
  return null;
}

function finalizeHint(line: ArtifactLine): string | undefined {
  if (line.kind === "design_spec" && line.finalized_versions.length === 0) {
    return "可尝试定稿；若 Story Spec 前置未满足，后端会返回 story_spec_not_confirmed";
  }
  return undefined;
}

function orderedSlots(line: ArtifactLine) {
  if (line.kind !== "design_spec") {
    return line.drafts;
  }
  return [...line.drafts].sort((left, right) => {
    const leftIndex = DESIGN_SLOT_ORDER.indexOf(left.slot_key);
    const rightIndex = DESIGN_SLOT_ORDER.indexOf(right.slot_key);
    return (leftIndex < 0 ? Number.MAX_SAFE_INTEGER : leftIndex) -
      (rightIndex < 0 ? Number.MAX_SAFE_INTEGER : rightIndex);
  });
}

function slotLabel(slotKey: DraftSlotKey) {
  return SLOT_LABELS[slotKey] ?? slotKey;
}

function finalizeErrorMessage(cause: unknown) {
  if (cause instanceof ApiRequestError && cause.code === "story_spec_not_confirmed") {
    return "需先定稿 Story Spec";
  }
  return cause instanceof Error && cause.message ? cause.message : "产物线定稿失败，请稍后重试";
}
