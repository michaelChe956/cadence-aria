// 从 GatePromptEntry.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// REQ-WSC-18（owner 2026-10-31 意见 1/2）——候选 Final Compile 失败的醒目呈现，
// 以及门卡 metadata `candidate_compile_failure` 的形状校验读取。此处只做
// 「读 metadata → 事实/渲染」，无状态、无副作用。
import { ChevronRight } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { DISCLOSURE_CHEVRON_CLASS, DISCLOSURE_SUMMARY_CLASS } from "../gate-visual-tokens";
import {
  gateCompileFailureAutoRewriteStatusCopy,
  type GateCompileFailureAutoRewriteInput,
} from "./gate-next-step";

/** REQ-WSC-18：候选编译失败的可呈现事实（缺省字段显式 null/[]，不猜）。 */
export type GateCandidateCompileFailureInfo = {
  code: string;
  message: string;
  detail: string | null;
  /** owner 2026-10-31 意见 2：design 可能需要一起重写的 owner 提醒。 */
  ownerHint: string | null;
  /** 重新生成（author 教学重驱）着重要点，与后端同源。 */
  rewriteGuidance: string[];
  /** owner 2026-10-31 追加裁定：自动重写两阶段事实（null=无该信息，不猜）。 */
  autoRewrite: GateCompileFailureAutoRewriteInput | null;
};

/**
 * REQ-WSC-18：门卡 metadata `candidate_compile_failure` 的形状校验读取
 * （{code,message,detail?,rewrite_guidance?,owner_hint?,auto_rewrite?}；
 * code/message 缺一即整条作废=null，不进「候选编译不过」态——fail-closed
 * 不猜。auto_rewrite.phase 非 running/stopped 亦按无该信息处理）。
 */
export function candidateCompileFailureFromEntry(
  entry: ChatEntry,
): GateCandidateCompileFailureInfo | null {
  const value = (entry.metadata as Record<string, unknown> | undefined)
    ?.candidate_compile_failure;
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const { code, message, detail, owner_hint, rewrite_guidance, auto_rewrite } =
    value as Record<string, unknown>;
  if (typeof code !== "string" || typeof message !== "string") {
    return null;
  }
  const autoRewrite =
    typeof auto_rewrite === "object" && auto_rewrite !== null
      ? parseAutoRewrite(auto_rewrite as Record<string, unknown>)
      : null;
  return {
    code,
    message,
    detail: typeof detail === "string" ? detail : null,
    ownerHint: typeof owner_hint === "string" ? owner_hint : null,
    rewriteGuidance: Array.isArray(rewrite_guidance)
      ? rewrite_guidance.filter((item): item is string => typeof item === "string")
      : [],
    autoRewrite,
  };
}

/** auto_rewrite 子对象校验：phase 合法才收；数字字段非 number 即 null（不自数）。 */
function parseAutoRewrite(
  value: Record<string, unknown>,
): GateCompileFailureAutoRewriteInput | null {
  const { phase, remaining, repeated } = value;
  if (phase !== "running" && phase !== "stopped") {
    return null;
  }
  return {
    phase,
    remaining: typeof remaining === "number" ? remaining : null,
    repeated: typeof repeated === "number" ? repeated : null,
  };
}

/**
 * REQ-WSC-18（owner 2026-10-31 意见 1/2）：候选 Final Compile 失败的醒目
 * 呈现——独立红框「候选无效：需重新生成」，不与普通 findings 混组、不藏
 * 折叠区；人话原因（message 完整不截断）、design 可能需重写提醒
 * （owner_hint）、重新生成要点（rewrite_guidance）直接可见；完整诊断原文
 * （detail）折叠可达、逐字不截断（owner 0009「看不到报错内容」修复）。
 */
export function GateCandidateCompileFailureBlock({
  failure,
}: {
  failure: GateCandidateCompileFailureInfo;
}) {
  return (
    <section
      data-testid="gate-compile-failure"
      className="space-y-2 rounded-md border-2 border-red-300 bg-red-50 px-3 py-2"
    >
      <div className="flex flex-wrap items-center gap-2">
        <span
          data-testid="gate-compile-failure-headline"
          className="text-sm font-bold text-red-800"
        >
          候选无效：Final Compile 未通过，需重新生成
        </span>
        <span
          data-testid="gate-compile-failure-code"
          className="aria-mono rounded bg-red-100 px-1.5 py-0.5 text-xs text-red-800"
        >
          {failure.code}
        </span>
      </div>
      <div
        data-testid="gate-compile-failure-message"
        className="text-sm text-slate-900"
      >
        {failure.message}
      </div>
      {/* owner 2026-10-31 追加裁定：阶段事实行——阶段一（自动重写进行中）
          陈述「还剩 N 次/无需手动推」；阶段二（同一环重复提前停）升级为
          加粗红字「自动重写无法解决，请修改 design」，均醒目不折叠。 */}
      {failure.autoRewrite ? (
        <div
          data-testid="gate-compile-failure-auto-rewrite"
          className={
            failure.autoRewrite.phase === "stopped"
              ? "text-sm font-bold text-red-800"
              : "text-sm font-medium text-slate-900"
          }
        >
          {gateCompileFailureAutoRewriteStatusCopy(failure.autoRewrite)}
        </div>
      ) : null}
      {failure.ownerHint ? (
        <div
          data-testid="gate-compile-failure-owner-hint"
          className="text-sm font-semibold text-red-700"
        >
          {failure.ownerHint}
        </div>
      ) : null}
      {failure.rewriteGuidance.length > 0 ? (
        <div
          data-testid="gate-compile-failure-rewrite-guidance"
          className="text-sm text-slate-900"
        >
          <div className="font-semibold">重新生成时着重注意：</div>
          <ul className="list-disc space-y-1 pl-5">
            {failure.rewriteGuidance.map((item, index) => (
              <li key={`${item}-${index}`}>{item}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {failure.detail ? (
        <details data-testid="gate-compile-failure-detail-disclosure">
          <summary
            className={`${DISCLOSURE_SUMMARY_CLASS} flex items-center gap-1 text-xs font-semibold text-slate-900`}
          >
            <ChevronRight className={DISCLOSURE_CHEVRON_CLASS} aria-hidden="true" />
            完整诊断原文
          </summary>
          <pre
            data-testid="gate-compile-failure-detail"
            className="aria-mono mt-1 overflow-x-auto whitespace-pre-wrap text-xs text-[var(--aria-ink)]"
          >
            {failure.detail}
          </pre>
        </details>
      ) : null}
    </section>
  );
}
