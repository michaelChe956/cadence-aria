// 从 CockpitInbox.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 门禁条动作面（门卡确认/反馈/终止、整组确认、compile recovery）与方案摘要块。
import { Check, ClipboardCopy, Play, RotateCcw, UserRound } from "lucide-react";
import { useState } from "react";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import {
  GATE_TERMINATE_BUTTON_LABEL,
  GATE_TERMINATE_CONFIRM_LABEL,
} from "../../../state/gate-prompt-copy";
import type {
  ArtifactVersionSummary,
  WorkItemPlanRepairReservation,
} from "../../../state/workspace-ws-store-types";
import {
  cockpitInboxItemSessionId,
  gateActionBlockCopy,
  type CockpitInboxItem,
} from "../../../state/workspace-cockpit-projection";
import {
  BTN_PRIMARY_CLASS,
  BTN_SECONDARY_CLASS,
  GATE_NESTED_BLOCK_CLASS,
} from "../gate-visual-tokens";
import { ConfirmTwiceButton } from "./ConfirmTwiceButton";
import { GateFeedbackEditor } from "./GateFeedbackEditor";

export function GateInboxActions({
  item,
  actions,
  artifactVersions,
  latestReviewSummary,
  repairReservation,
}: {
  item: CockpitInboxItem;
  actions: CockpitActionFacade;
  artifactVersions: readonly ArtifactVersionSummary[];
  latestReviewSummary: string | null;
  repairReservation: WorkItemPlanRepairReservation | null;
}) {
  const [feedback, setFeedback] = useState("");
  const actionBlockReason = item.gate?.action_block_reason ?? null;
  // F-21：终止专属判据（缺省回退通用判据）——plan 会话停在 human_confirm 的
  // context blocker/author 失败/缺相位门允许终止（矩阵+引擎只校验 stage+flow）。
  // F-21：terminate_block_reason 显式 null=终止放行；仅缺省（undefined，旧投影
  // 形态）才回退通用判据——?? 会把 null 吞成回退。
  const terminateBlockReason =
    item.gate !== null && item.gate.terminate_block_reason !== undefined
      ? item.gate.terminate_block_reason
      : actionBlockReason;
  // REQ-PCG-01/02：批次确认与 compile recovery 是 durable node 门（无 typed
  // turn/snapshot），动作面独立于既有 typed/legacy 分流——阻断时只呈现原因，
  // 写动作一律不露出（F-30 终态守卫）。
  const nodeGateKind =
    item.gate?.kind === "batch_confirm" || item.gate?.kind === "compile_recovery"
      ? item.gate.kind
      : null;
  if (nodeGateKind !== null) {
    if (actionBlockReason !== null) {
      return (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
          {gateActionBlockCopy(actionBlockReason)}
        </p>
      );
    }
    return nodeGateKind === "batch_confirm" ? (
      <BatchConfirmActions actions={actions} />
    ) : (
      <CompileRecoveryActions actions={actions} />
    );
  }

  // C2（REQ-HGC-02/plan T2 ④，F-54 §4.2）：phase_mismatch 通用相位提示行删除
  // （与门卡同源裁决）——不渲染误导文案，终止通路照常。
  if (actionBlockReason && actionBlockReason !== "phase_mismatch" && terminateBlockReason) {
    return (
      <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
        {gateActionBlockCopy(actionBlockReason)}
      </p>
    );
  }

  // F-31（v37 复验 #2）：story/design author 门（产物确认阶段）——HTTP confirm
  // 通路，确认语义为定稿；评审可选（review_available=reviewerEnabled）。
  const authorGate = item.gate?.stage === "author_confirm";
  const typed = item.gate?.flow_kind === "single_candidate";
  const typedGateNeedsNewCommand =
    typed &&
    typeof item.gate?.turn?.command_id !== "string" &&
    repairReservation?.owner_session_id === cockpitInboxItemSessionId(item.id) &&
    (repairReservation.state === "reserved" || repairReservation.state === "provider_started");
  const summary = gateSummary(artifactVersions, latestReviewSummary);

  return (
    <div className="mt-3 space-y-3">
      {summary ? <GateSummary {...summary} /> : null}
      <div className="flex flex-wrap gap-2">
        {actionBlockReason !== null && actionBlockReason !== "phase_mismatch" ? (
          // F-21：phase_mismatch 的 plan 门终止专属放行——只露终止，说明行替代
          // 确认/反馈（相位纪律维持）。
          <p className="w-full text-xs text-slate-500">
            {gateActionBlockCopy(actionBlockReason)}，可终止后重新发起
          </p>
        ) : null}
        {typed && actionBlockReason === null ? (
          <>
            {typedGateNeedsNewCommand ? (
              <p className="w-full text-xs text-slate-500">
                未同步门命令，将以新命令提交
              </p>
            ) : null}
            <GateFeedbackEditor
              multiline
              value={feedback}
              onChange={setFeedback}
              onSubmit={actions.feedback}
            />
          </>
        ) : null}
        {/* L1（REQ-RET-02）：legacy 门「采纳建议并返修」（request-change）发送面删除。 */}
        {actionBlockReason === null ? (
          <button
            type="button"
            onClick={actions.confirm}
            className={BTN_PRIMARY_CLASS}
          >
            <Check className="h-4 w-4" aria-hidden="true" />
            {authorGate ? "确认定稿" : "确认"}
          </button>
        ) : null}
        {/* F-31（v37 复验 #2）：author 门与主区门卡动作面对齐——reviewer 启用
            （投影 review_available）追加「确认并评审」（with_review=true，服务端
            接管进入评审轮）；未启用不露出，与主区同源判据。 */}
        {authorGate && item.gate?.review_available && actionBlockReason === null ? (
          <button
            type="button"
            onClick={actions.confirmReview}
            className={BTN_SECONDARY_CLASS}
          >
            确认并评审
          </button>
        ) : null}
        {/* v40 复验 #3：author 门 review 已完成（存在未被修订取代的最新 review
            报告，同主区 latestReviewReport 判据）时补齐第四钮——与主区/产物
            审核面板「采纳 Review 意见」同款：门面 adoptReview 预填修订反馈+
            切回对话视图。不与 review_available（可发起评审）耦合：评审完成后
            即使 reviewer 已关，既有报告仍可采纳。无 review 结果不露出。 */}
        {authorGate && latestReviewSummary !== null && actionBlockReason === null ? (
          <button
            type="button"
            onClick={actions.adoptReview}
            className={BTN_SECONDARY_CLASS}
          >
            <ClipboardCopy className="h-4 w-4" aria-hidden="true" />
            采纳 Review 意见
          </button>
        ) : null}
        <ConfirmTwiceButton
          variant="ghost"
          label={GATE_TERMINATE_BUTTON_LABEL}
          confirmLabel={GATE_TERMINATE_CONFIRM_LABEL}
          onConfirm={actions.terminate}
        />
      </div>
    </div>
  );
}

/**
 * REQ-PCG-01：整组 Work Item Draft 确认门动作面——[确认整组]（HTTP confirm 通路，
 * 门面 confirmBatch）+ [终止]（既有 WS abandon 二次确认）。确认语义是整组 Draft，
 * 不重新暴露 legacy 逐段决策，也不提供批量勾选（批量 runner 只发 WS confirm 帧）。
 */
function BatchConfirmActions({ actions }: { actions: CockpitActionFacade }) {
  return (
    <div className="mt-3 flex flex-wrap gap-2">
      <button
        type="button"
        onClick={() => {
          void actions.confirmBatch();
        }}
        className={BTN_PRIMARY_CLASS}
      >
        <Check className="h-4 w-4" aria-hidden="true" />
        确认整组
      </button>
      <ConfirmTwiceButton
        variant="ghost"
        label={GATE_TERMINATE_BUTTON_LABEL}
        confirmLabel={GATE_TERMINATE_CONFIRM_LABEL}
        onConfirm={actions.terminate}
      />
    </div>
  );
}

/**
 * REQ-PCG-02：Final Compile recovery 动作面——既有 `WorkItemPlanCompileRecoveryAction`
 * 三动作原样派发（不映射为 confirm/feedback/abandon）；`human_triage` 可携带原因
 * （wire 可选 reason，能力对齐 legacy WorkItemPlanStagedPanel 的动作集合）。
 */
function CompileRecoveryActions({ actions }: { actions: CockpitActionFacade }) {
  const [triageReason, setTriageReason] = useState("");
  // F-50 视觉 v2 §1：recovery 三动作=次操作描边（确认类才上实心主操作）。
  return (
    <div className="mt-3 space-y-2">
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => {
            void actions.recoverCompile("continue");
          }}
          className={BTN_SECONDARY_CLASS}
        >
          <Play className="h-4 w-4" aria-hidden="true" />
          继续
        </button>
        <button
          type="button"
          onClick={() => {
            void actions.recoverCompile("abort_and_rollback");
          }}
          className={BTN_SECONDARY_CLASS}
        >
          <RotateCcw className="h-4 w-4" aria-hidden="true" />
          放弃并回滚
        </button>
        <button
          type="button"
          onClick={() => {
            void actions.recoverCompile("human_triage", triageReason.trim() || undefined);
          }}
          className={BTN_SECONDARY_CLASS}
        >
          <UserRound className="h-4 w-4" aria-hidden="true" />
          转人工
        </button>
      </div>
      <label className="flex min-h-11 items-center gap-2 text-xs font-medium text-slate-700">
        转人工原因（可选）
        <input
          type="text"
          value={triageReason}
          onChange={(event) => setTriageReason(event.target.value)}
          className="min-h-9 flex-1 rounded-md border border-slate-300 bg-gray-50 px-2 text-xs text-slate-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400"
        />
      </label>
    </div>
  );
}

function gateSummary(
  versions: readonly ArtifactVersionSummary[],
  reviewSummary: string | null,
): { title: string; version: number; verdict: string | null; changes: string; review: string | null } | null {
  const current = versions.find((version) => version.is_current) ??
    [...versions].sort((left, right) => right.version - left.version)[0];
  const markdown = current?.markdown?.trim();
  if (!current || !markdown) {
    return null;
  }
  const lines = markdown.split("\n").map((line) => line.trim()).filter(Boolean);
  const title = lines.find((line) => line.startsWith("#"))?.replace(/^#+\s*/u, "") ?? "当前方案";
  const changes = lines
    .filter((line) => /^[-*]\s+/u.test(line))
    .map((line) => line.replace(/^[-*]\s+/u, ""))
    .slice(0, 3)
    .join("；");
  return { title, version: current.version, verdict: current.review_verdict ?? null, changes, review: reviewSummary };
}

function GateSummary({
  title,
  version,
  verdict,
  changes,
  review,
}: {
  title: string;
  version: number;
  verdict: string | null;
  changes: string;
  review: string | null;
}) {
  const verdictLabel = verdict === "pass" ? "审核通过" : verdict === "revise" ? "需要返修" : verdict === "needs_human" ? "需要人工判断" : "待审核";
  return (
    <section aria-label="等待确认的内容" className={GATE_NESTED_BLOCK_CLASS}>
      <p className="text-xs font-semibold text-slate-500">等待确认的内容</p>
      <p className="mt-1 truncate text-sm font-semibold text-slate-900">{title}</p>
      <p className="mt-1 text-xs text-slate-500">版本 {version} · {verdictLabel}</p>
      {changes ? <p className="mt-2 text-xs leading-5 text-slate-700">{changes}</p> : null}
      {review ? <p className="mt-2 text-xs leading-5 text-slate-500">{review}</p> : null}
    </section>
  );
}
