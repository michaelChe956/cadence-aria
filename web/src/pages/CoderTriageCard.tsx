// P2-d（REQ-CRO-09）：coder 分诊卡——`reason_code ∈ {coding_output_human_triage,
// coding_operational_blocker}` 的 blocked gate 专用渲染（块 2 设计 §2.1/§3.4）。
// 三段大白话（buildTriageNarrative）＋三按钮（由 available_actions 驱动，
// 旧门无 retry_coding_with_context 时退化为两按钮，X2 兼容在途 attempt）＋
// 「技术详情」折叠（GateMetadata/原始 description/逐条 finding）内二级折叠
// 「高级：验证处理」（原 GatePanel 的验证处理与命令证据 JSX 原样迁入，
// 入口默认隐藏但保留全部既有行为；owner 裁定不移除）。
// GateMetadata/VerificationTriageSection 从 CodingWorkspaceControls 迁出/抽取至此，
// 使依赖方向保持单向（Controls → Card），generic 面板与分诊卡共用同一份渲染。
import { useState, type ReactNode } from "react";
import type {
  CodingGateRequired,
  VerificationCommandEvidence,
  VerificationTriageRecord,
} from "../api/types";
import { buildTriageNarrative } from "../state/coding-triage-narrative";
import type { CodingPendingGate } from "../state/coding-workspace-store";

export const CODER_TRIAGE_RETRY_BUTTON_LABEL = "让 AI 再试一次";
export const CODER_TRIAGE_RETRY_WITH_CONTEXT_BUTTON_LABEL = "我来说明情况，让 AI 继续";
export const CODER_TRIAGE_ABORT_BUTTON_LABEL = "终止";

const CONTEXT_CHAR_LIMIT = 2000;
/** 分诊卡文件列表默认展示前 3 个，其余收进折叠明细。 */
const VISIBLE_CHANGED_FILES = 3;

/** 原始 description/reason/raw/evidence 技术元数据（自 CodingWorkspaceControls 迁出，零行为变化）。 */
export function GateMetadata({ gate }: { gate: CodingPendingGate }) {
  const rows = [
    gate.reason_code ? ["reason", gate.reason_code] : null,
    gate.raw_provider_output_ref ? ["raw", gate.raw_provider_output_ref] : null,
    gate.evidence_refs?.length ? ["evidence", gate.evidence_refs.join(", ")] : null,
  ].filter(Boolean) as [string, string][];
  if (rows.length === 0) {
    return null;
  }
  return (
    <dl className="mt-1 grid gap-0.5 text-xs text-amber-800">
      {rows.map(([label, value]) => (
        <div key={label} className="grid min-w-0 grid-cols-[4.5rem_minmax(0,1fr)] gap-2">
          <dt className="font-semibold">{label}</dt>
          <dd className="min-w-0 break-words font-mono">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

/**
 * 验证处理旁路面板＋命令证据区（自 CodingWorkspaceControls 的 GatePanel 抽取，
 * JSX 零行为变化）。资格判定（isVerificationTriageEligibleGate）留给调用方：
 * generic 面板按白名单包裹；分诊卡的两种 reason_code 均在白名单内，
 * 在「高级：验证处理」二级折叠内直接渲染。
 */
export function VerificationTriageSection({
  gate,
  verificationTriage = null,
  onEnterVerificationTriage,
  commandEvidence = null,
  onRerunPlannedCommand,
}: {
  gate: CodingPendingGate;
  verificationTriage?: VerificationTriageRecord | null;
  onEnterVerificationTriage?: (gateId: string) => void;
  commandEvidence?: VerificationCommandEvidence | null;
  onRerunPlannedCommand?: (checkId: string) => void;
}) {
  return (
    <>
      <div data-testid="coding-verification-triage" className="mt-1 text-xs text-amber-900">
        <div className="font-semibold">验证处理</div>
        {verificationTriage ? (
          <div data-testid="coding-verification-triage-status">
            状态：
            {verificationTriage.status === "approved"
              ? "已批准"
              : verificationTriage.status === "rejected"
                ? "已拒绝"
                : "待处理"}
            {verificationTriage.status === "approved" &&
            verificationTriage.conclusion !== "approve_plan_revision"
              ? "（finding 已由验证处理覆盖）"
              : null}
          </div>
        ) : (
          <button
            type="button"
            data-testid="coding-verification-triage-enter"
            disabled={!onEnterVerificationTriage}
            onClick={() => onEnterVerificationTriage?.(gate.gate_id)}
            className="mt-1 inline-flex h-7 items-center rounded-md border border-amber-300 bg-white px-2 font-semibold hover:bg-amber-100 disabled:opacity-60"
          >
            转入验证处理
          </button>
        )}
      </div>
      {commandEvidence ? (
        <div
          data-testid="coding-command-evidence"
          className="mt-1 grid gap-0.5 text-xs text-amber-900"
        >
          <div className="font-semibold">命令证据</div>
          <div className="font-mono">
            计划命令：
            {commandEvidence.planned_command ??
              commandEvidence.planned_manual_instruction ??
              "未记录"}
          </div>
          <div className="font-mono">
            实际命令：{commandEvidence.actual_command ?? "未记录"}
          </div>
          <div className="font-mono">
            实际 cwd：{commandEvidence.actual_cwd ?? "未记录"}
          </div>
          <div className="font-mono">
            退出码：{commandEvidence.exit_code ?? "未记录"}
          </div>
          {commandEvidence.mismatch ? (
            <div data-testid="coding-command-evidence-mismatch" className="font-semibold">
              实际执行命令与计划不一致
            </div>
          ) : null}
          {commandEvidence.mismatch ? (
            <button
              type="button"
              data-testid="coding-rerun-planned-command"
              disabled={!onRerunPlannedCommand}
              onClick={() => onRerunPlannedCommand?.(commandEvidence.check_id)}
              className="mt-1 inline-flex h-7 w-fit items-center rounded-md border border-amber-300 bg-white px-2 font-semibold hover:bg-amber-100 disabled:opacity-60"
            >
              重跑原计划命令
            </button>
          ) : null}
        </div>
      ) : null}
    </>
  );
}

function ChangedFileList({ files }: { files: string[] }) {
  if (files.length === 0) {
    return null;
  }
  const visible = files.slice(0, VISIBLE_CHANGED_FILES);
  const rest = files.slice(VISIBLE_CHANGED_FILES);
  return (
    <details className="mt-0.5">
      <summary className="cursor-pointer text-amber-800">
        改动文件（{files.length}）
      </summary>
      <ul className="mt-0.5 grid gap-0.5 font-mono">
        {visible.map((file) => (
          <li key={file} className="min-w-0 break-words">
            {file}
          </li>
        ))}
        {rest.length > 0 ? (
          <li className="text-amber-700">……其余 {rest.length} 个见原始输出</li>
        ) : null}
      </ul>
    </details>
  );
}

export function CoderTriageCard({
  gate,
  onRespond,
  automationManaged = false,
  onOpenTermination,
  verificationTriage = null,
  onEnterVerificationTriage,
  commandEvidence = null,
  onRerunPlannedCommand,
}: {
  gate: CodingPendingGate;
  onRespond: (gateId: string, actionId: string, extraContext?: string) => void;
  /** 批 2 T4（REQ-WIGA-09）：托管判据（T3 同源投影 owner=server）——门动作
   * 集合不变，但 abort 执行语义改整 plan 统一终止（路由二次确认面）。 */
  automationManaged?: boolean;
  onOpenTermination?: () => void;
  /** 原样透传自 GatePanel：验证处理旁路面板（收进二级折叠）。 */
  verificationTriage?: VerificationTriageRecord | null;
  onEnterVerificationTriage?: (gateId: string) => void;
  commandEvidence?: VerificationCommandEvidence | null;
  onRerunPlannedCommand?: (checkId: string) => void;
}) {
  const [contextOpen, setContextOpen] = useState(false);
  const [contextText, setContextText] = useState("");

  const narrative = buildTriageNarrative(gate.triage_summary, gate.description);
  const submitting = gate.submitting === true;
  const gateErrorCode = gate.errorCode ?? null;
  // 三按钮由 available_actions 驱动而非硬编码 action_id（块 2 设计 §3.4）：
  // 旧门未下发 retry_coding_with_context 时第二按钮不渲染（X2）。
  const retryAction =
    gate.available_actions.find((action) => action.action_type === "retry_coding") ?? null;
  const retryWithContextAction =
    gate.available_actions.find(
      (action) => action.action_type === "retry_coding_with_context",
    ) ?? null;
  const abortAction =
    gate.available_actions.find((action) => action.action_type === "abort") ?? null;

  const trimmedContext = contextText.trim();
  const contextTooLong = contextText.length > CONTEXT_CHAR_LIMIT;
  const contextEmpty = !trimmedContext;
  const contextHint = contextTooLong
    ? "说明不能超过 2000 字"
    : contextOpen && contextEmpty
      ? "请先写一两句话"
      : null;

  function submitRetryWithContext() {
    if (!retryWithContextAction || contextEmpty || contextTooLong) {
      return;
    }
    onRespond(gate.gate_id, retryWithContextAction.action_id, trimmedContext);
  }

  const actionButtonClass =
    "inline-flex h-8 items-center justify-center rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-900 hover:bg-amber-100 disabled:opacity-60";

  const contextForm: ReactNode = contextOpen && retryWithContextAction ? (
    <div className="grid w-full gap-1 md:w-72">
      <textarea
        aria-label="补充说明"
        value={contextText}
        onChange={(event) => {
          setContextText(event.target.value);
        }}
        rows={2}
        maxLength={CONTEXT_CHAR_LIMIT + 100}
        placeholder="用一两句话说明情况；该说明会随「让 AI 继续」一并带给 AI"
        className="min-h-14 w-full resize-y rounded-md border border-amber-300 bg-white px-2 py-1.5 text-xs text-amber-950 placeholder:text-amber-700"
      />
      {contextHint ? (
        <div data-testid="coder-triage-context-hint" className="text-xs font-semibold text-[var(--aria-danger)]">
          {contextHint}
        </div>
      ) : null}
      <button
        type="button"
        data-testid="coder-triage-context-submit"
        disabled={submitting || contextEmpty || contextTooLong}
        onClick={submitRetryWithContext}
        className={`${actionButtonClass} w-fit`}
      >
        {submitting ? "处理中" : "提交"}
      </button>
    </div>
  ) : null;

  return (
    <div
      data-testid="coder-triage-card"
      className="border-t border-amber-200 bg-amber-50 px-3 py-2"
    >
      <div className="flex min-w-0 flex-col gap-2 md:flex-row md:items-start md:justify-between">
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-amber-900">{gate.title}</div>
          <div data-testid="coder-triage-problem" className="mt-1 text-xs text-amber-900">
            <span className="font-semibold">AI 遇到什么问题：</span>
            {narrative.problem}
          </div>
          {narrative.did ? (
            <div data-testid="coder-triage-did" className="mt-1 text-xs text-amber-900">
              <span className="font-semibold">AI 做了什么：</span>
              {narrative.did.headline}
              <ChangedFileList files={narrative.did.changedFiles} />
            </div>
          ) : null}
          {narrative.unverified.length > 0 ? (
            <div data-testid="coder-triage-unverified" className="mt-1 text-xs text-amber-900">
              <div className="font-semibold">没能验证什么：</div>
              <ul className="mt-0.5 grid list-disc gap-0.5 pl-4">
                {narrative.unverified.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            </div>
          ) : null}
          <details data-testid="coder-triage-technical-details" className="mt-1">
            <summary className="cursor-pointer text-xs font-semibold text-amber-900">
              技术详情
            </summary>
            <div className="mt-0.5 text-xs text-amber-800">{gate.description}</div>
            {narrative.findings.length > 0 ? (
              <ul className="mt-0.5 grid gap-0.5 text-xs text-amber-800">
                {narrative.findings.map((finding) => (
                  <li key={`${finding.reasonCode}:${finding.message}`}>
                    <span className="font-mono">{finding.reasonCode}</span>
                    <span>：{finding.message}</span>
                  </li>
                ))}
              </ul>
            ) : null}
            <GateMetadata gate={gate} />
            <details data-testid="coder-triage-advanced-verification" className="mt-1">
              <summary className="cursor-pointer text-xs font-semibold text-amber-900">
                高级：验证处理
              </summary>
              <VerificationTriageSection
                gate={gate}
                verificationTriage={verificationTriage}
                onEnterVerificationTriage={onEnterVerificationTriage}
                commandEvidence={commandEvidence}
                onRerunPlannedCommand={onRerunPlannedCommand}
              />
            </details>
          </details>
          {gateErrorCode ? (
            <div className="mt-1 text-xs font-semibold text-[var(--aria-danger)]">
              {gateErrorCode}
            </div>
          ) : null}
        </div>
        <div
          data-testid="coder-triage-actions"
          className="flex shrink-0 flex-col items-start gap-2"
        >
          <div className="flex shrink-0 flex-wrap gap-2">
            {retryAction ? (
              <button
                type="button"
                disabled={submitting}
                onClick={() => onRespond(gate.gate_id, retryAction.action_id, undefined)}
                className={actionButtonClass}
              >
                {submitting ? "处理中" : CODER_TRIAGE_RETRY_BUTTON_LABEL}
              </button>
            ) : null}
            {retryWithContextAction ? (
              <button
                type="button"
                disabled={submitting}
                onClick={() => setContextOpen(true)}
                className={actionButtonClass}
              >
                {CODER_TRIAGE_RETRY_WITH_CONTEXT_BUTTON_LABEL}
              </button>
            ) : null}
            {abortAction ? (
              <button
                type="button"
                disabled={submitting}
                onClick={() => {
                  // 批 2 T4：托管下 Abort 走整 plan 统一终止；手动保持单 attempt。
                  if (automationManaged && onOpenTermination) {
                    onOpenTermination();
                    return;
                  }
                  onRespond(gate.gate_id, abortAction.action_id, undefined);
                }}
                className={actionButtonClass}
              >
                {submitting ? "处理中" : CODER_TRIAGE_ABORT_BUTTON_LABEL}
              </button>
            ) : null}
          </div>
          {contextForm}
        </div>
      </div>
    </div>
  );
}
