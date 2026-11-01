// 从 CockpitInbox.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 硬错误条渲染（协议/引擎错误 alert、诊断折叠、trust 未信任指引卡）。
import {
  ChevronRight,
  ClipboardCopy,
  RotateCcw,
  ShieldCheck,
} from "lucide-react";
import { useState } from "react";
import type { WorkspaceProviderName } from "../../../api/types";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import {
  GATE_TERMINATE_BUTTON_LABEL,
  GATE_TERMINATE_CONFIRM_LABEL,
} from "../../../state/gate-prompt-copy";
import type { LeaseDiagnosticsEvent } from "../../../state/lease-diagnostics";
import {
  PROTOCOL_ERROR_DETAILS_LABEL,
  PROTOCOL_ERROR_NO_RETRY_NOTE,
  protocolErrorCopy,
} from "../../../state/protocol-error-copy";
import {
  isStaleDriverLeaseItem,
  type CockpitInboxItem,
} from "../../../state/workspace-cockpit-projection";
import { LeaseDiagnosticsSummary } from "../LeaseDiagnosticsSummary";
import {
  BTN_SECONDARY_CLASS,
  DISCLOSURE_CHEVRON_CLASS,
  DISCLOSURE_SUMMARY_CLASS,
} from "../gate-visual-tokens";
import { ConfirmTwiceButton } from "./ConfirmTwiceButton";
import { KIND_GLYPH } from "./cockpitInboxKindTokens";

/** 硬错误条的长诊断折叠（原拒绝串/长指导原文；F-50 §4.1-10）。 */
function HardErrorDiagnostics({ item }: { item: CockpitInboxItem }) {
  return (
    <details data-testid="cockpit-inbox-error-details" className="group mt-1">
      <summary
        className={`${DISCLOSURE_SUMMARY_CLASS} flex items-center gap-1 text-xs font-medium text-slate-600`}
      >
        <ChevronRight className={DISCLOSURE_CHEVRON_CLASS} aria-hidden="true" />
        {PROTOCOL_ERROR_DETAILS_LABEL}
      </summary>
      <p className="aria-mono mt-1 break-words text-xs leading-4 text-slate-600">
        {item.summary}
      </p>
      {/* 裁决 5：不可安全重放的原因作为可见文本（title 悬停不可达）。 */}
      {item.source !== "advance" ? (
        <p className="mt-1 text-xs text-slate-600">{PROTOCOL_ERROR_NO_RETRY_NOTE}</p>
      ) : null}
    </details>
  );
}

/** trust 指引卡显示名（wire 标签 → 卡面用名）。 */
const TRUST_PROVIDER_DISPLAY_NAMES: Record<string, string> = {
  kimi_code: "Kimi",
  codex: "Codex",
};

/**
 * trust 未信任指引卡（owner 2026-10-12 直批冻结设计）：两行简洁说明＋
 * 可复制命令窗——第一行直白告知该目录尚未被哪些 provider 信任（按实际
 * 单复数）；第二行怎么做（一键登记或复制命令自行登记后重新生成）；
 * 命令文本来自后端同源 `trust_missing_guidance` 帧（Kimi 单行 shell／
 * Codex TOML 片段），前端不解析平铺 error 串。无结构化命令（旧链文本
 * 识别/用户值冲突分型）不渲染命令窗，说明与登记按钮仍在。
 */
function TrustMissingPanel({ trust }: { trust: NonNullable<CockpitInboxItem["trustMissing"]> }) {
  const [copied, setCopied] = useState(false);
  const providers = trust.providers ?? [trust.provider];
  const providerLabel = providers
    .map((provider) => TRUST_PROVIDER_DISPLAY_NAMES[provider] ?? provider)
    .join(" 与 ");
  const rootLabel = trust.canonicalRoot ? `（${trust.canonicalRoot}）` : "";
  const command = trust.commandMarkdown ?? null;
  const copyCommand = async () => {
    if (command === null) {
      return;
    }
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // 剪贴板不可用（权限/非安全上下文）：保持未复制态，不伪称成功。
    }
  };
  return (
    <div data-testid="cockpit-trust-guidance" className="mt-2 space-y-2">
      <p
        data-testid="cockpit-trust-missing-headline"
        className="text-sm font-medium text-slate-900"
      >
        该目录{rootLabel}尚未被 {providerLabel} 信任
      </p>
      <p data-testid="cockpit-trust-missing-action" className="text-xs leading-4 text-slate-600">
        点击下方按钮一键登记；或复制命令自行登记后重新生成
      </p>
      {command !== null ? (
        <div
          data-testid="cockpit-trust-command-window"
          className="rounded-md border border-slate-300 bg-slate-50"
        >
          <div className="flex items-center justify-between gap-2 border-b border-slate-200 px-2 py-1">
            <span className="text-[11px] font-medium text-slate-500">登记命令</span>
            <button
              type="button"
              data-testid="cockpit-trust-command-copy"
              onClick={() => {
                void copyCommand();
              }}
              className="inline-flex items-center gap-1 rounded border border-slate-300 bg-white px-2 py-0.5 text-[11px] font-semibold text-slate-700 hover:bg-slate-100"
            >
              <ClipboardCopy className="h-3 w-3" aria-hidden="true" />
              {copied ? "已复制" : "复制命令"}
            </button>
          </div>
          <pre className="aria-mono overflow-x-auto px-2 py-1.5 text-[11px] leading-4 text-slate-800">
            {command}
          </pre>
        </div>
      ) : null}
    </div>
  );
}
/**
 * F-50 §4.1-7/8/10：协议/引擎错误条——紧凑 alert（role=alert），不再复刻门禁卡
 * 高度；原文与长诊断进折叠详情；无安全重放命令（source ≠ advance）的重试不渲染
 * （灰置会被误读为暂时忙），advance 来源保留「重试」。
 */
export function HardErrorInboxRow({
  item,
  pulse,
  actions,
  actionable,
  onRetry,
  onRetakeLease,
  leaseEvents,
}: {
  item: CockpitInboxItem;
  pulse: boolean;
  actions?: CockpitActionFacade;
  actionable: boolean;
  onRetry?: (item: CockpitInboxItem) => void;
  onRetakeLease?: () => void;
  /** REQ-DLS-03：STALE 错误条引用的最近租约转移事件（缺省不渲染）。 */
  leaseEvents: readonly LeaseDiagnosticsEvent[] | null;
}) {
  const Glyph = KIND_GLYPH.hard_error;
  return (
    <article
      data-testid="cockpit-inbox-item-hard_error"
      data-pulse={pulse}
      role="alert"
      className={[
        // F-50 视觉 v2 §3：错误条中性底——红只上图标+mono 码（不再红底整条）。
        "rounded-lg border border-slate-200 bg-white px-3 py-2",
        pulse ? "motion-safe:animate-pulse ring-2 ring-[var(--aria-danger)]" : "",
      ].join(" ")}
    >
      <div className="flex items-start gap-2">
        <Glyph className="mt-0.5 h-4 w-4 shrink-0 text-red-600" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-baseline gap-x-2">
            <span className="text-sm font-semibold text-slate-900">{item.title}</span>
            {item.protocolErrorCode ? (
              <span
                data-testid="cockpit-inbox-error-code"
                className="aria-mono text-xs font-normal text-red-600"
              >
                {item.protocolErrorCode}
              </span>
            ) : null}
          </p>
          {/* F-50 裁决 6：已知码配中文正文；没有可负责任的译文时不编（正文缺省，
              原文已在折叠详情）。 */}
          {item.protocolErrorCode && protocolErrorCopy(item.protocolErrorCode).body ? (
            <p className="mt-1 text-xs leading-4 text-slate-600">
              {protocolErrorCopy(item.protocolErrorCode).body}
            </p>
          ) : null}
          {/* REQ-DLS-03 规范句：STALE 错误条引用最近租约转移事件（可定案
              偷窃者身份）；非 STALE 硬错误或诊断缺省（端点失败静默降级）不渲染。 */}
          {isStaleDriverLeaseItem(item) && leaseEvents ? (
            <LeaseDiagnosticsSummary events={leaseEvents} />
          ) : null}
          {item.inlineError ? (
            <p className="aria-mono mt-1 break-words text-xs text-red-600">
              {item.inlineError.code} · {item.inlineError.message}
            </p>
          ) : null}
          {/* 指引卡简化（owner 2026-10-12 直批冻结设计）：trust 未信任
              指引卡重排为「两行简洁说明＋可复制命令窗」——冗长细节
              （原拒绝串/长指导）收进「查看诊断详情」折叠并移到卡尾；
              非 trust 错误条维持折叠在正文的既有布局。 */}
          {item.trustMissing ? null : <HardErrorDiagnostics item={item} />}
          {item.trustMissing && !item.trustRegistered ? (
            <TrustMissingPanel trust={item.trustMissing} />
          ) : null}
          {/* 「登记并重试」按钮（owner 2026-10-12 直批）：trust 缺失错误卡
              按稳定码识别（投影已解析 provider）；显式授权走 ensure 登记链，
              成功回执后切「已登记，请显式重新生成」——不自动重新生成。 */}
          {item.trustMissing ? (
            item.trustRegistered ? (
              <p
                data-testid="cockpit-trust-registered-hint"
                role="status"
                className="mt-1 text-xs font-medium text-emerald-700"
              >
                已登记，请显式重新生成
              </p>
            ) : null
          ) : null}
          {actions && actionable && item.trustMissing && !item.trustRegistered ? (
            <div className="mt-2 flex flex-wrap gap-2">
              <button
                type="button"
                data-testid="cockpit-trust-register-button"
                onClick={() => {
                  const provider = item.trustMissing?.provider;
                  if (provider !== undefined) {
                    actions.registerTrustEntry?.(provider as WorkspaceProviderName);
                  }
                }}
                className={BTN_SECONDARY_CLASS}
              >
                <ShieldCheck className="h-4 w-4" aria-hidden="true" />
                登记并重试
              </button>
            </div>
          ) : null}
          {item.trustMissing ? (
            <div className="mt-2">
              <HardErrorDiagnostics item={item} />
            </div>
          ) : null}
          {actions && actionable ? (
            <div className="mt-2 flex flex-wrap gap-2">
              {isStaleDriverLeaseItem(item) && onRetakeLease ? (
                <ConfirmTwiceButton
                  label="重新接管"
                  confirmLabel="确认重新接管"
                  onConfirm={onRetakeLease}
                />
              ) : null}
              {item.source === "advance" ? (
                <button
                  type="button"
                  onClick={() => onRetry?.(item)}
                  className={BTN_SECONDARY_CLASS}
                >
                  <RotateCcw className="h-4 w-4" aria-hidden="true" />
                  重试推进
                </button>
              ) : null}
              {/* 裁决 4：terminate → abandon_human_gate（门级），命名显式带作用域。 */}
              <ConfirmTwiceButton
                variant="ghost"
                label={GATE_TERMINATE_BUTTON_LABEL}
                confirmLabel={GATE_TERMINATE_CONFIRM_LABEL}
                onConfirm={actions.terminate}
              />
            </div>
          ) : null}
        </div>
      </div>
    </article>
  );
}
