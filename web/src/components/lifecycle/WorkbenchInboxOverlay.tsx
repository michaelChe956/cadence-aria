import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CircleAlert,
  CircleCheck,
  ClipboardList,
  AlertTriangle,
  ExternalLink,
  ListChecks,
  Play,
  RotateCcw,
} from "lucide-react";
import type {
  CodingAttemptAddress,
  CodingAttemptSnapshotResponse,
} from "../../api/types";
import {
  ApiRequestError,
  AUTOMATION_PLAN_TERMINATION_REQUIRED,
  parseAutomationTerminationIdentity,
  postCodingGateResponse,
} from "../../api/client";
import {
  codingAttentionLocationItem,
  useCockpitAttentionTargetOpener,
  useCockpitShellInbox,
} from "../cockpit/CockpitShell";
import { AutomationTerminationConfirmSurface } from "../chat-workspace/cockpit/CockpitInboxWaitingCards";
import { ChoiceInboxCard } from "../chat-workspace/cockpit/CockpitInbox";
import { CoderTriageCard } from "../../pages/CoderTriageCard";
import { isTriageCardGate } from "../../pages/CodingWorkspaceControls";
import { useCockpitChoiceRespond } from "../../pages/useCockpitChoiceRespond";
import {
  useInboxAttemptDetails,
  type InboxAttemptDetail,
  type InboxAttemptDetailsApi,
} from "../../hooks/useInboxAttemptDetails";
import type { ChoiceResponsePayload } from "../../state/chat-entries";
import { notifyLifecycleInvalidated } from "../../state/lifecycle-workbench-store";
import { useCodingWorkspaceStore } from "../../state/coding-workspace-store";
import {
  dedupeCodingChoiceInboxItems,
  type CockpitInboxItem,
} from "../../state/workspace-cockpit-projection";
import { WorkbenchOverlayDrawer } from "../shared/WorkbenchOverlayDrawer";

/**
 * UI-3（owner 拍板）：overlay=inbox 的待处理统一抽屉——把既有 Cockpit 待处理
 * 收件箱（CockpitShell 观察者目录：gate/choice 投影 + lifecycle
 * coding_attention_items + 恢复/错误项原分组）提升为共享宿主。目录与计数
 * 复用同一投影（不新建第二套待办模型）；操作详情按「attention 是目录，
 * snapshot 是操作详情」在行内补齐：coding_action 行展开时按完整 Attempt
 * 地址拉 snapshot（同 Attempt 复用一次），choice 复用 ChoiceInboxCard +
 * useCockpitChoiceRespond（既有 REST 协议），分诊门复用 CoderTriageCard +
 * postCodingGateResponse（expected_version 取服务端 attempt_version），
 * 托管终止走整 plan 终止确认面。收起（open=false）驻留不卸载：未提交
 * 表单/展开态保留。打开/查看/关闭 ≠ 完成，完成事实以 lifecycle 目录
 * 消失为准（不引入本地「已读即删除」）。
 */

/** search 定位字段（UI-1 冻结契约的 inbox 消费面）。 */
export type InboxFocusFields = {
  planId?: string;
  attemptId?: string;
  workItemId?: string;
  gateId?: string;
  choiceId?: string;
};

const KIND_GLYPH = {
  gate: ClipboardList,
  stopped: CircleAlert,
  hard_error: AlertTriangle,
  choice: ListChecks,
  info: CircleCheck,
  c1_recovery: RotateCcw,
  sc_failed: CircleAlert,
  lc_bootstrap: Play,
  lc_resume_recovery: RotateCcw,
  coding_action: ListChecks,
} as const;

const KIND_CLASS: Record<CockpitInboxItem["kind"], string> = {
  gate: "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
  stopped: "rounded-lg border border-slate-200 bg-slate-50 px-3 py-2",
  hard_error: "rounded-lg border border-red-200 bg-red-50 px-3 py-2",
  choice: "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
  info: "rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2",
  c1_recovery: "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
  sc_failed: "rounded-lg border border-slate-200 bg-slate-50 px-3 py-2",
  lc_bootstrap: "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
  lc_resume_recovery: "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
  coding_action: "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
};

export function WorkbenchInboxOverlay({
  focus,
  open,
  onClose,
}: {
  focus: InboxFocusFields;
  /** false=收起驻留（保留内容与未提交表单，退出交互/无障碍树）。 */
  open: boolean;
  onClose: () => void;
}) {
  const description = [
    focus.attemptId,
    focus.gateId,
    focus.choiceId,
  ]
    .filter((part): part is string => Boolean(part))
    .join(" · ");
  return (
    <WorkbenchOverlayDrawer
      label="待处理事项"
      title="待处理事项"
      description={description || undefined}
      testId="workbench-inbox-overlay"
      onClose={onClose}
      open={open}
    >
      <WorkbenchInboxContent focus={focus} />
    </WorkbenchOverlayDrawer>
  );
}

function itemMatchesFocus(item: CockpitInboxItem, focus: InboxFocusFields): boolean {
  const info = item.attentionInfo ?? null;
  if (info === null) {
    return false;
  }
  if (focus.attemptId !== undefined) {
    if (info.attemptId !== focus.attemptId) {
      return false;
    }
  } else if (focus.planId !== undefined && info.planId !== focus.planId) {
    return false;
  }
  if (focus.choiceId !== undefined && info.focus?.choice_id !== focus.choiceId) {
    return false;
  }
  if (focus.gateId !== undefined && info.focus?.gate_id !== focus.gateId) {
    return false;
  }
  return true;
}

function focusSignature(focus: InboxFocusFields): string {
  return [
    focus.planId ?? "",
    focus.attemptId ?? "",
    focus.workItemId ?? "",
    focus.gateId ?? "",
    focus.choiceId ?? "",
  ].join("|");
}

function WorkbenchInboxContent({ focus }: { focus: InboxFocusFields }) {
  const directory = useCockpitShellInbox();
  // 同一 choice 双来源只计一次：coding 就地卡优先，同规范身份的目录行丢弃。
  const items = useMemo(() => dedupeCodingChoiceInboxItems(directory), [directory]);
  // 详情缓存整抽屉一份：同 Attempt 的 snapshot/attempt_version 复用一次。
  const details = useInboxAttemptDetails();
  const [expandedIds, setExpandedIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  // 详情面板首次展开才挂载（打开/选中时才拉取），收起后驻留（保留未提交
  // 表单）——与 expandedIds 分离，避免未展开行提前拉 snapshot。
  const [mountedDetailIds, setMountedDetailIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const expandItem = useCallback((itemId: string) => {
    setExpandedIds((previous) => new Set([...previous, itemId]));
    setMountedDetailIds((previous) => new Set([...previous, itemId]));
  }, []);
  const focusAppliedRef = useRef("");
  const signature = focusSignature(focus);
  const hasFocus =
    focus.planId !== undefined ||
    focus.attemptId !== undefined ||
    focus.workItemId !== undefined ||
    focus.gateId !== undefined ||
    focus.choiceId !== undefined;

  // 通知/深链聚焦：定位字段变化时展开并高亮匹配行（每签名一次；用户随后
  // 可手动收起，不因渲染反复强开）。
  useEffect(() => {
    if (!hasFocus || focusAppliedRef.current === signature) {
      return;
    }
    focusAppliedRef.current = signature;
    const matched = items.filter((item) => itemMatchesFocus(item, focus));
    if (matched.length === 0) {
      return;
    }
    for (const item of matched) {
      expandItem(item.id);
    }
  }, [expandItem, focus, hasFocus, items, signature]);

  const toggleExpanded = useCallback((itemId: string) => {
    setExpandedIds((previous) => {
      const next = new Set(previous);
      if (next.has(itemId)) {
        next.delete(itemId);
      } else {
        next.add(itemId);
      }
      return next;
    });
    // 首次交互即挂载详情（打开/选中时拉取）；收起仅隐藏不卸载。
    setMountedDetailIds((previous) =>
      previous.has(itemId) ? previous : new Set([...previous, itemId]),
    );
  }, []);

  const connectionItems = items.filter((item) => item.kind === "hard_error");
  const humanItems = items.filter(
    (item) => item.kind !== "hard_error" && item.kind !== "info",
  );
  const infoItems = items.filter((item) => item.kind === "info");

  return (
    <div
      data-testid="workbench-inbox-content"
      className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain px-3 py-2"
    >
      {/* 托管终止二次确认面（全局 coding store 驱动；分诊卡 abort 428 时打开）。 */}
      <AutomationTerminationConfirmSurface />
      {items.length === 0 ? (
        <p
          data-testid="workbench-inbox-empty"
          className="px-2 py-3 text-xs text-[var(--aria-ink-muted)]"
        >
          暂无待处理项
        </p>
      ) : (
        <div className="space-y-4">
          {connectionItems.length > 0 ? (
            <InboxGroup label="连接问题">
              {connectionItems.map((item) => (
                <InboxDirectoryRow
                  key={item.id}
                  item={item}
                  expanded={expandedIds.has(item.id)}
                  detailMounted={mountedDetailIds.has(item.id)}
                  focusMatched={hasFocus && itemMatchesFocus(item, focus)}
                  onToggle={toggleExpanded}
                  details={details}
                />
              ))}
            </InboxGroup>
          ) : null}
          {humanItems.length > 0 ? (
            <InboxGroup label="需要人工处理">
              {humanItems.map((item) => (
                <InboxDirectoryRow
                  key={item.id}
                  item={item}
                  expanded={expandedIds.has(item.id)}
                  detailMounted={mountedDetailIds.has(item.id)}
                  focusMatched={hasFocus && itemMatchesFocus(item, focus)}
                  onToggle={toggleExpanded}
                  details={details}
                />
              ))}
            </InboxGroup>
          ) : null}
          {infoItems.length > 0 ? (
            <InboxGroup label="进度信息">
              {infoItems.map((item) => (
                <InboxDirectoryRow
                  key={item.id}
                  item={item}
                  expanded={expandedIds.has(item.id)}
                  detailMounted={mountedDetailIds.has(item.id)}
                  focusMatched={hasFocus && itemMatchesFocus(item, focus)}
                  onToggle={toggleExpanded}
                  details={details}
                />
              ))}
            </InboxGroup>
          ) : null}
        </div>
      )}
    </div>
  );
}

function InboxGroup({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <h3 className="text-xs font-medium uppercase tracking-wider text-[var(--aria-ink-muted)]">
        {label}
      </h3>
      {children}
    </div>
  );
}

function InboxDirectoryRow({
  item,
  expanded,
  detailMounted,
  focusMatched,
  onToggle,
  details,
}: {
  item: CockpitInboxItem;
  expanded: boolean;
  /** 详情面板是否已挂载（首次展开后保持挂载；未展开过不拉取）。 */
  detailMounted: boolean;
  focusMatched: boolean;
  onToggle: (itemId: string) => void;
  details: InboxAttemptDetailsApi;
}) {
  const Glyph = KIND_GLYPH[item.kind];
  const scopeParts = [
    item.attentionInfo?.repositoryName ?? null,
    item.attentionInfo?.projectId ?? null,
    item.attentionInfo?.issueId ?? null,
    item.attentionInfo?.attemptId ?? null,
  ].filter((part): part is string => Boolean(part));
  return (
    <article
      data-testid={`workbench-inbox-item-${item.kind}`}
      data-focus-match={focusMatched ? "true" : undefined}
      data-state={expanded ? "expanded" : "collapsed"}
      className={[
        KIND_CLASS[item.kind],
        focusMatched ? "ring-2 ring-amber-400" : "",
      ].join(" ")}
    >
      <div className="flex items-start gap-2">
        <Glyph className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="min-w-0 truncate text-sm font-semibold text-slate-900">
            {item.title}
          </p>
          {item.summary ? (
            <p className="mt-1 break-words text-xs leading-4 text-slate-600">
              {item.summary}
            </p>
          ) : null}
          {scopeParts.length > 0 ? (
            <p className="aria-mono mt-1 break-all text-[11px] text-slate-500">
              {scopeParts.join(" · ")}
            </p>
          ) : null}
          {item.createdAt ? (
            <p className="mt-0.5 text-[11px] text-slate-400">{item.createdAt}</p>
          ) : null}
          {item.kind === "coding_action" && item.attentionInfo ? (
            <button
              type="button"
              data-testid="workbench-inbox-item-toggle"
              onClick={() => onToggle(item.id)}
              className="mt-2 inline-flex min-h-9 items-center gap-1 rounded-md border border-amber-300 bg-white px-2.5 py-1 text-xs font-semibold text-amber-900 transition-colors duration-200 hover:bg-amber-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-500"
            >
              {expanded ? "收起详情" : "处理"}
            </button>
          ) : null}
        </div>
      </div>
      {/* 展开过的详情面板保持挂载（仅隐藏）——收起保留未提交表单。 */}
      {item.kind === "coding_action" && item.attentionInfo && detailMounted ? (
        <div hidden={!expanded} className="mt-2 space-y-2">
          <CodingAttentionDetail item={item} details={details} />
        </div>
      ) : null}
    </article>
  );
}

/**
 * coding_action 行内操作详情：attention 是目录，snapshot 是操作详情——
 * 展开时按完整 Attempt 地址拉 snapshot + 服务端 attempt_version；choice 复用
 * ChoiceInboxCard（既有 REST），分诊门复用 CoderTriageCard（postCodingGate
 * Response，expected_version 只取服务端版本事实）；其余目标只导航不执行。
 */
function CodingAttentionDetail({
  item,
  details,
}: {
  item: CockpitInboxItem;
  details: InboxAttemptDetailsApi;
}) {
  const info = item.attentionInfo;
  const openTarget = useCockpitAttentionTargetOpener();
  const address: CodingAttemptAddress | null =
    info !== null &&
    info !== undefined &&
    info.target.type === "coding_attempt" &&
    info.attemptId !== null
      ? {
          projectId: info.projectId,
          issueId: info.issueId,
          attemptId: info.attemptId,
        }
      : null;
  const detail = address !== null ? details.get(address.attemptId) : undefined;

  // 打开/选中时拉取：detail 缺失（首次展开或 lifecycle invalidation 清空后）
  // 即 ensure；同 Attempt 在途/就绪复用；失败保留目录项、显式重试。
  useEffect(() => {
    if (address !== null && detail === undefined) {
      details.ensure(address);
    }
  }, [address, detail, details]);

  const openTargetButton =
    info !== null && info !== undefined && openTarget !== null ? (
      <button
        type="button"
        data-testid="workbench-inbox-open-target"
        onClick={() => openTarget(codingAttentionLocationItem(info))}
        className="inline-flex min-h-9 items-center gap-1 rounded-md border border-amber-300 bg-white px-2.5 py-1 text-xs font-semibold text-amber-900 transition-colors duration-200 hover:bg-amber-100"
      >
        <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
        打开目标工作区
      </button>
    ) : null;

  if (info === null || info === undefined) {
    return null;
  }
  if (address === null) {
    // plan/workspace_session 目标：不在抽屉内执行，提供原统一导航下钻。
    return (
      <div className="space-y-1">
        <p className="text-xs text-slate-500">
          该事项在对应工作区处理（抽屉内不执行动作）
        </p>
        {openTargetButton}
      </div>
    );
  }
  if (detail === undefined || detail.status === "loading") {
    return (
      <p
        data-testid="workbench-inbox-detail-loading"
        className="text-xs text-[var(--aria-ink-muted)]"
      >
        正在拉取操作详情…
      </p>
    );
  }
  if (detail.status === "error") {
    return (
      <div
        data-testid="workbench-inbox-detail-error"
        role="alert"
        className="space-y-1"
      >
        <p className="text-xs font-semibold text-[var(--aria-danger)]">
          操作详情拉取失败：{detail.error}
        </p>
        <button
          type="button"
          onClick={() => details.retry(address.attemptId)}
          className="inline-flex h-8 items-center rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-900 hover:bg-amber-100"
        >
          重试
        </button>
      </div>
    );
  }
  const snapshot = detail.snapshot;
  if (snapshot === null) {
    return null;
  }
  return (
    <div data-testid="workbench-inbox-detail" className="space-y-2">
      <AttentionChoiceDetail item={item} address={address} snapshot={snapshot} />
      <AttentionGateDetail item={item} address={address} detail={detail} />
      {openTargetButton}
    </div>
  );
}

/** choice：按 focus 精确匹配 snapshot.pending_choices，复用 ChoiceInboxCard。 */
function AttentionChoiceDetail({
  item,
  address,
  snapshot,
}: {
  item: CockpitInboxItem;
  address: CodingAttemptAddress;
  snapshot: CodingAttemptSnapshotResponse;
}) {
  const info = item.attentionInfo;
  const { choiceCommands, handleChoiceRespond } = useCockpitChoiceRespond();
  const ledgerIssueRef = useRef(new Map<string, string>());
  const notifiedRef = useRef(new Set<string>());
  // delivered 回执按 issue 走既有 lifecycle invalidation（每 key 一次），
  // 不在渲染期派发。
  useEffect(() => {
    for (const [ledgerKey, record] of Object.entries(choiceCommands)) {
      if (record.status === "delivered" && !notifiedRef.current.has(ledgerKey)) {
        notifiedRef.current.add(ledgerKey);
        const issueId = ledgerIssueRef.current.get(ledgerKey) ?? "";
        if (issueId !== "") {
          notifyLifecycleInvalidated(issueId);
        }
      }
    }
  }, [choiceCommands]);


  if (info === null || info === undefined) {
    return null;
  }
  const choiceId = info.focus?.choice_id ?? null;
  if (choiceId === null) {
    return null;
  }
  const gate =
    snapshot.pending_choices.find(
      (candidate) => candidate.choice_id === choiceId,
    ) ?? null;

  // 回执后走既有 lifecycle invalidation：delivered 事实按 issue 广播一次，
  // 目录行由观察者下一轮刷新收敛（打开/查看/关闭本身不删除条目）。
  const handleRespond = (
    target: CockpitInboxItem,
    payload: ChoiceResponsePayload,
  ) => {
    const choice = target.choice;
    if (choice !== null) {
      const scopeId =
        choice.sessionId ?? choice.attemptAddress?.attemptId ?? "";
      ledgerIssueRef.current.set(
        `${scopeId}:${choice.choiceId}`,
        choice.attemptAddress?.issueId ?? "",
      );
    }
    handleChoiceRespond(target, payload);
  };
  if (gate === null || gate.status !== "open") {
    return (
      <p className="text-xs text-[var(--aria-ink-muted)]">
        该选择请求已处理或已失效，等待目录刷新
      </p>
    );
  }
  const scopeId = address.attemptId;
  const key = `${scopeId}:${gate.choice_id}`;
  const record = choiceCommands[key];
  if (record?.status === "delivered") {
    return (
      <p
        data-testid="workbench-inbox-choice-delivered"
        className="text-xs font-medium text-emerald-700"
      >
        答案已送达，等待事实收敛
      </p>
    );
  }
  const questions =
    gate.questions && gate.questions.length > 0
      ? gate.questions
      : [
          {
            id: "default",
            prompt: gate.prompt,
            options: gate.options,
            allow_multiple: gate.allow_multiple,
            allow_free_text: gate.allow_free_text,
          },
        ];
  const choiceItem: CockpitInboxItem = {
    id: `coding_attention:${address.projectId}:${address.issueId}:detail-choice:${gate.choice_id}`,
    kind: "choice",
    severity: 2,
    title: "选择请求待作答",
    summary: gate.prompt,
    triage: false,
    source: "choice",
    createdAt: gate.created_at ?? null,
    gate: null,
    inlineError:
      record?.status === "error"
        ? {
            code: record.errorCode ?? "choice_response_failed",
            message: record.errorMessage ?? "应答未送达，可重试",
          }
        : null,
    choice: {
      // 显式作用域：目录/驾驶舱会话无关，scope 取 attemptId（台账键唯一）。
      sessionId: address.attemptId,
      choiceId: gate.choice_id,
      prompt: gate.prompt,
      expectedRunId: gate.expected_run_id ?? null,
      questions,
      allowMultiple: gate.allow_multiple,
      allowFreeText: gate.allow_free_text,
      source: "coding",
      attemptAddress: address,
      status:
        record?.status === "submitting" ||
        record?.status === "resolving" ||
        record?.status === "expired"
          ? record.status
          : "open",
    },
  };
  return (
    <ChoiceInboxCard item={choiceItem} onChoiceRespond={handleRespond} />
  );
}

/** 分诊门：focus 精确匹配 pending_gates；triage 门复用 CoderTriageCard 直接操作。 */
function AttentionGateDetail({
  item,
  address,
  detail,
}: {
  item: CockpitInboxItem;
  address: CodingAttemptAddress;
  detail: InboxAttemptDetail;
}) {
  const info = item.attentionInfo;
  const [actions, setActions] = useState<
    Record<
      string,
      { status: "submitting" | "error" | "done"; message: string | null }
    >
  >({});

  if (info === null || info === undefined) {
    return null;
  }
  const gateId = info.focus?.gate_id ?? null;
  if (gateId === null || detail.snapshot === null) {
    return null;
  }
  const gate =
    detail.snapshot.pending_gates.find(
      (candidate) => candidate.gate_id === gateId,
    ) ?? null;
  if (gate === null) {
    return (
      <p className="text-xs text-[var(--aria-ink-muted)]">
        该门已处理或已关闭，等待目录刷新
      </p>
    );
  }

  const actionKey = `${address.attemptId}:${gate.gate_id}`;
  // 该门全部动作的本地命令态（key `${attemptId}:${gateId}:${actionId}`）。
  const gateStates = Object.entries(actions).filter(([key]) =>
    key.startsWith(`${address.attemptId}:${gate.gate_id}:`),
  );
  const submitting = gateStates.some(([, state]) => state.status === "submitting");
  const errorState = gateStates.find(([, state]) => state.status === "error");
  const doneState = gateStates.some(([, state]) => state.status === "done");

  const respond = async (
    targetGateId: string,
    actionId: string,
    extraContext?: string,
  ) => {
    // expected_version 只取服务端版本事实（listVerificationTriageRecords 的
    // attempt_version）；缺失 fail-closed，不用 rework_count 冒充。
    const version = detail.attemptVersion;
    const key = `${actionKey}:${actionId}`;
    if (version === null) {
      setActions((previous) => ({
        ...previous,
        [key]: {
          status: "error",
          message: "服务端版本未就绪（expected_version 缺失），请稍后重试",
        },
      }));
      return;
    }
    setActions((previous) => ({
      ...previous,
      [key]: { status: "submitting", message: null },
    }));
    try {
      await postCodingGateResponse(address, {
        // 同命令同 payload 重试：command_id 按 (attempt,gate,action) 派生稳定。
        command_id: `cmd-inbox-gate-${address.attemptId}-${targetGateId}-${actionId}`,
        gate_id: targetGateId,
        action_id: actionId,
        extra_context: extraContext ?? null,
        expected_version: version,
      });
      setActions((previous) => ({
        ...previous,
        [key]: { status: "done", message: null },
      }));
      // 回执后走既有 lifecycle invalidation（目录 + 本抽屉详情缓存重拉）。
      notifyLifecycleInvalidated(address.issueId);
    } catch (error) {
      if (
        error instanceof ApiRequestError &&
        error.code === AUTOMATION_PLAN_TERMINATION_REQUIRED
      ) {
        // 托管终止走整 plan 终止确认面（零请求副作用，身份以服务端为准）。
        const identity = parseAutomationTerminationIdentity(error.details);
        useCodingWorkspaceStore.getState().openTerminationConfirm({
          projectId: address.projectId,
          issueId: address.issueId,
          enrollmentId: identity?.enrollment_id ?? null,
          policyRevision: identity?.policy_revision ?? null,
        });
        return;
      }
      setActions((previous) => ({
        ...previous,
        [key]: {
          status: "error",
          message:
            error instanceof Error && error.message !== ""
              ? error.message
              : "门禁应答失败，可重试",
        },
      }));
    }
  };

  if (isTriageCardGate(gate)) {
    return (
      <div className="space-y-1">
        <CoderTriageCard
          gate={{
            ...gate,
            submitting: submitting || undefined,
            errorCode: errorState ? (errorState[1].message ?? null) : null,
          }}
          onRespond={(targetGateId, actionId, extraContext) => {
            void respond(targetGateId, actionId, extraContext);
          }}
        />
        {/* 错误经 gate.errorCode 在卡内就地呈现，不重复渲染第二份。 */}
        {doneState ? (
          <p role="status" className="text-xs font-medium text-emerald-700">
            已提交，等待事实收敛
          </p>
        ) : null}
      </div>
    );
  }
  // 非 triage 门（blocked/staged/final confirm 等）：只读呈现 available_actions，
  // 不在抽屉复制第二套门表单；操作去 Coding 工作区（行尾「打开目标工作区」）。
  return (
    <div className="space-y-1">
      <p className="text-xs font-medium text-slate-700">{gate.title}</p>
      <p className="text-xs text-slate-600">{gate.description}</p>
      {gate.available_actions.length > 0 ? (
        <p className="text-[11px] text-slate-500">
          可用动作：{gate.available_actions.map((action) => action.label).join(" / ")}
        </p>
      ) : null}
    </div>
  );
}
