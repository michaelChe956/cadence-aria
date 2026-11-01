// F-39/F-49 结论卡与门卡共用的 findings 呈现（单一事实源）：severity 分级、分组
// 标题、徽章与行的样式都只此一份，避免门卡按「同款样式」抄出第二套后漂移。
//
// C2（REQ-HGC-03 场景 1/F-52 §三）双轨分类透明：每行以 effective class
// （class_hint 优先的策略语义）为主显、severity（用户语义）为标注；分组键
// 同步改为 effective class（severity 保底）——severity=advisory 而 class_hint
// 为 repairable/human_required 时不再被归入「可选建议」造成「建议不阻断」误导。
import type { ChatEntry } from "../../state/chat-entries";

export type ReviewOwnerOption = {
  label: string;
  consequence: string;
};

export type ReviewFinding = {
  severity: string;
  message: string;
  evidence?: string;
  required_action?: string;
  /** 策略分类提示（wire snake_case：repairable/human_required/advisory）。 */
  class_hint?: string;
  /** REQ-WSC-11：面向 owner 的一句话人话提问（仅 needs_human 携带）。 */
  owner_question?: string;
  /** REQ-WSC-11：owner 可选的互斥处置项（2–3 项）。 */
  owner_options?: ReviewOwnerOption[];
  /**
   * REQ-WSC-12：WI 范围判定——actionable=可由本 Work Item 修订收敛；
   * advisory=范围外事实，只作「仅供参考」提示。缺失 = actionable（与解析层
   * fail-safe 同口径，兼容旧记录）。
   */
  disposition?: "actionable" | "advisory";
  /**
   * REQ-WSC-13：平台算的内容指纹（sha256(severity+\x1+message) 前 16 hex）。
   * 排除动作的唯一身份；缺失（旧记录）= 不渲染「排除此项」（fail-closed）。
   */
  findingKey?: string;
  /** REQ-WSC-12：finding 指到的具体 Work Item（指不到时缺席）。 */
  targetWorkItemId?: string;
};

/**
 * REQ-WSC-13 §6 契约：门面 feedback 的扩展签名——排除项随
 * HumanGateFeedback 出站（camelCase `excludedFindingIds`；排除-only 帧
 * feedback 为空串）。少参函数天然可赋值本类型：门面未升级到双参签名时按
 * 「发送失败」呈现（fail-closed，不静默丢排除项）。
 */
export type FeedbackSenderWithOptions = (
  feedback: string,
  options?: { excludedFindingIds?: string[] },
) => boolean | void;

export type ReviewOwnerGuidance = {
  question: string;
  options: ReviewOwnerOption[];
};

/**
 * REQ-WSC-11：从 findings 里取第一条完整的人话裁决（问题 + 至少两个互斥
 * 选项）。缺任一字段、或有效选项不足 2 条即视为没有——与服务端 fail-closed
 * 下限（`options.len() >= 2`）同口径，前端不拼接猜测：没有事实就不画人话
 * 区，退回既有技术呈现。
 */
export function reviewOwnerGuidance(findings: ReviewFinding[]): ReviewOwnerGuidance | null {
  for (const finding of findings) {
    const question = finding.owner_question?.trim();
    const options = (finding.owner_options ?? []).filter((option) => {
      const label = option?.label?.trim();
      const consequence = option?.consequence?.trim();
      return Boolean(label && consequence);
    });
    if (question && options.length >= 2) {
      return { question, options };
    }
  }
  return null;
}

/**
 * REQ-WSC-11（owner 可读的人工裁决）：结论卡与门卡共用的唯一人话区块——
 * 「问题 + 选项名 + 后果 + 一句作答指引」。答案由 owner 手写补充指令完成，
 * 本块不出站、不生成命令；选项名原文就是可粘贴的补充指令。
 */
export function ReviewOwnerGuidanceBlock({ guidance }: { guidance: ReviewOwnerGuidance }) {
  return (
    <section
      className="space-y-2 rounded-md border border-amber-200 bg-white px-3 py-2"
      data-testid="owner-guidance"
    >
      <div className="text-xs font-semibold text-amber-900">需要你决定一件事</div>
      <div className="text-sm font-medium text-slate-900" data-testid="owner-question">
        {guidance.question}
      </div>
      <ul className="space-y-2">
        {guidance.options.map((option, index) => (
          <li
            key={`${option.label}-${index}`}
            className="rounded border border-[var(--aria-line)] bg-white px-2 py-1.5"
            data-testid="owner-option"
          >
            <div className="text-sm font-medium text-[var(--aria-ink)]">{option.label}</div>
            <div className="text-xs text-[var(--aria-ink-muted)]">{option.consequence}</div>
          </li>
        ))}
      </ul>
      <div className="text-xs text-slate-600" data-testid="owner-option-guidance">
        选一个，把选项名原文作为补充指令发回去即可（例如：「{guidance.options[0].label}」）。
      </div>
    </section>
  );
}

export function reviewFindingsFromEntry(entry: ChatEntry): ReviewFinding[] {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const findings = Array.isArray(metadata?.findings) ? metadata.findings : [];
  return findings.filter(isReviewFinding);
}

function isReviewFinding(value: unknown): value is ReviewFinding {
  if (!value || typeof value !== "object") {
    return false;
  }
  const finding = value as Record<string, unknown>;
  return typeof finding.severity === "string" && typeof finding.message === "string";
}

/** 需人工处理（高）：blocking / must_fix；其余（含 suggestion）视为建议级。 */
export function isRequiredFindingSeverity(severity: string) {
  return severity === "blocking" || severity === "must_fix";
}

/** C2（REQ-HGC-03）：阻断判据以 effective class 为准，severity 保底。 */
export function isBlockingFinding(finding: ReviewFinding) {
  if (finding.class_hint) {
    return finding.class_hint !== "advisory";
  }
  return isRequiredFindingSeverity(finding.severity);
}

/** REQ-WSC-12：advisory = 仅提示（不阻断、不计数、不要求作者执行）；缺失 = actionable。 */
export function isAdvisoryDisposition(finding: ReviewFinding): boolean {
  return finding.disposition === "advisory";
}

/**
 * REQ-WSC-12 §2 平台硬规则的前端镜像：只有 actionable 计入「需要解决」的
 * 计数与分组——disposition=advisory 即使 class_hint/severity 判阻断也不进
 * 需处理面（展示侧不依赖模型把 disposition 标对）。
 */
export function isActionableBlockingFinding(finding: ReviewFinding): boolean {
  return !isAdvisoryDisposition(finding) && isBlockingFinding(finding);
}

const CLASS_HINT_LABELS: Record<string, string> = {
  repairable: "需处理",
  human_required: "需人工",
  advisory: "建议",
};

export function ReviewFindingGroups({
  findings,
  excludedFindingKeys,
  onExcludeFinding,
  onRevokeExclusion,
  revokingFindingKeys,
  revokingAll,
}: {
  findings: ReviewFinding[];
  /** REQ-WSC-13：owner 已成功出站排除的 finding key 集（本地即时反馈）。 */
  excludedFindingKeys?: ReadonlySet<string>;
  /** REQ-WSC-13：排除发送器（门卡接线）；缺席（结论卡等只读面）不渲染排除按钮。 */
  onExcludeFinding?: (finding: ReviewFinding) => boolean;
  /** REQ-WSC-13/14（k3 P0-2C）：撤销排除发送器（REST，按平台 findingKey）；缺席不渲染撤销按钮。 */
  onRevokeExclusion?: (findingKey: string) => void;
  /** 撤销在途的 key（行级禁用+「撤销中…」，防重复点击）。 */
  revokingFindingKeys?: ReadonlySet<string>;
  /** 全量撤销在途（期间所有行级撤销按钮一并禁用）。 */
  revokingAll?: boolean;
}) {
  // REQ-WSC-12 §2 前端镜像：advisory（范围外事实）整组落「仅供参考（不阻断）」，
  // 不参与「需要解决」的阻断分组——即使 class_hint/severity 判阻断。
  const advisoryFindings = findings.filter(isAdvisoryDisposition);
  const actionableFindings = findings.filter((finding) => !isAdvisoryDisposition(finding));
  const requiredFindings = actionableFindings.filter((finding) => isBlockingFinding(finding));
  const optionalFindings = actionableFindings.filter((finding) => !isBlockingFinding(finding));
  return (
    <>
      {requiredFindings.length > 0 ? (
        <FindingGroup
          title="需要解决"
          tone="required"
          findings={requiredFindings}
          excludedFindingKeys={excludedFindingKeys}
          onExcludeFinding={onExcludeFinding}
          onRevokeExclusion={onRevokeExclusion}
          revokingFindingKeys={revokingFindingKeys}
          revokingAll={revokingAll}
        />
      ) : null}
      {optionalFindings.length > 0 ? (
        <FindingGroup
          title="可选建议"
          tone="optional"
          findings={optionalFindings}
          excludedFindingKeys={excludedFindingKeys}
          onExcludeFinding={onExcludeFinding}
          onRevokeExclusion={onRevokeExclusion}
          revokingFindingKeys={revokingFindingKeys}
          revokingAll={revokingAll}
        />
      ) : null}
      {advisoryFindings.length > 0 ? (
        <FindingGroup
          title="仅供参考（不阻断）"
          tone="optional"
          findings={advisoryFindings}
          advisory={true}
          excludedFindingKeys={excludedFindingKeys}
          onExcludeFinding={onExcludeFinding}
          onRevokeExclusion={onRevokeExclusion}
          revokingFindingKeys={revokingFindingKeys}
          revokingAll={revokingAll}
        />
      ) : null}
    </>
  );
}

function FindingGroup({
  title,
  tone,
  findings,
  advisory = false,
  excludedFindingKeys,
  onExcludeFinding,
  onRevokeExclusion,
  revokingFindingKeys,
  revokingAll,
}: {
  title: string;
  tone: "required" | "optional";
  findings: ReviewFinding[];
  /** REQ-WSC-12：本组是否为 advisory（仅供参考）呈现。 */
  advisory?: boolean;
  excludedFindingKeys?: ReadonlySet<string>;
  onExcludeFinding?: (finding: ReviewFinding) => boolean;
  onRevokeExclusion?: (findingKey: string) => void;
  revokingFindingKeys?: ReadonlySet<string>;
  revokingAll?: boolean;
}) {
  const titleClass = tone === "required" ? "text-amber-900" : "text-[var(--aria-ink-muted)]";
  const borderClass = tone === "required" ? "border-amber-200" : "border-[var(--aria-line)]";
  return (
    <section className={`space-y-2 rounded-md border ${borderClass} bg-white px-3 py-2`}>
      <div
        data-testid="review-finding-group-title"
        className={`text-xs font-semibold ${titleClass}`}
      >
        {title}
      </div>
      <div className="space-y-2">
        {findings.map((finding, index) => (
          <FindingRow
            key={`${finding.severity}-${index}`}
            finding={finding}
            advisory={advisory}
            excluded={Boolean(
              finding.findingKey && excludedFindingKeys?.has(finding.findingKey),
            )}
            onExcludeFinding={onExcludeFinding}
            onRevokeExclusion={onRevokeExclusion}
            revoking={
              revokingAll === true ||
              (finding.findingKey !== undefined &&
                revokingFindingKeys?.has(finding.findingKey) === true)
            }
          />
        ))}
      </div>
    </section>
  );
}

function FindingRow({
  finding,
  advisory,
  excluded,
  onExcludeFinding,
  onRevokeExclusion,
  revoking = false,
}: {
  finding: ReviewFinding;
  advisory: boolean;
  excluded: boolean;
  onExcludeFinding?: (finding: ReviewFinding) => boolean;
  /** REQ-WSC-13/14（k3 P0-2C）：撤销排除发送器（已排除行专属入口）。 */
  onRevokeExclusion?: (findingKey: string) => void;
  /** 撤销在途：按钮禁用并显示「撤销中…」（防重复点击）。 */
  revoking?: boolean;
}) {
  return (
    <div className="space-y-1" data-testid="review-finding">
      <div className="flex flex-wrap items-center gap-2">
        {advisory ? <AdvisoryBadge /> : null}
        <EffectiveClassBadge classHint={finding.class_hint} severity={finding.severity} />
        <SeverityBadge severity={finding.severity} />
        <div className="text-sm font-medium text-[var(--aria-ink)]">{finding.message}</div>
      </div>
      {finding.evidence ? (
        <div className="text-xs text-[var(--aria-ink-muted)]">{finding.evidence}</div>
      ) : null}
      {finding.required_action ? (
        <div
          data-testid={advisory ? "advisory-required-action" : undefined}
          // REQ-WSC-12：advisory 的 required_action 只作提示——不得加粗成
          // 「要求作者执行」的祈使样式。
          className={
            advisory
              ? "text-xs text-[var(--aria-ink-muted)]"
              : "text-xs font-medium text-[var(--aria-ink)]"
          }
        >
          {finding.required_action}
        </div>
      ) : null}
      {/* REQ-WSC-13（owner 裁定取舍 2）：逐条「排除此项」——身份用平台算的
          findingKey；旧记录无 key 不渲染按钮（fail-closed：无法 durable 引用）。
          REQ-WSC-13/14（k3 P0-2C）：已排除行旁给「撤销排除」——排错后能重新
          提出为 actionable；在途禁用防重复出站。 */}
      {excluded ? (
        <div className="flex flex-wrap items-center gap-2">
          <span
            data-testid="finding-excluded-badge"
            className="inline-flex shrink-0 items-center rounded border border-emerald-200 bg-emerald-50 px-1.5 py-0.5 text-[11px] font-semibold text-emerald-700"
          >
            已排除
          </span>
          {onRevokeExclusion && finding.findingKey ? (
            <button
              type="button"
              data-testid="finding-revoke-exclusion-button"
              disabled={revoking}
              onClick={() => {
                if (finding.findingKey) {
                  onRevokeExclusion(finding.findingKey);
                }
              }}
              className="text-xs font-medium text-slate-600 underline underline-offset-2 transition-colors hover:text-slate-900 disabled:pointer-events-none disabled:opacity-50"
            >
              {revoking ? "撤销中…" : "撤销排除"}
            </button>
          ) : null}
        </div>
      ) : onExcludeFinding && finding.findingKey ? (
        <button
          type="button"
          data-testid="finding-exclude-button"
          onClick={() => onExcludeFinding(finding)}
          className="text-xs font-medium text-slate-600 underline underline-offset-2 transition-colors hover:text-slate-900"
        >
          排除此项
        </button>
      ) : null}
    </div>
  );
}

/** REQ-WSC-12：advisory（非本 Work Item 范围事实）行的「仅供参考」徽章。 */
function AdvisoryBadge() {
  return (
    <span
      data-testid="finding-advisory-badge"
      title="disposition: advisory（不阻断、不计数、不要求作者执行）"
      className="inline-flex shrink-0 rounded border border-slate-200 bg-slate-50 px-1.5 py-0.5 text-[11px] font-semibold text-slate-600"
    >
      仅供参考
    </span>
  );
}

/** C2（REQ-HGC-03）：effective class 主显徽章；缺席时退 severity 语义。 */
function EffectiveClassBadge({ classHint, severity }: { classHint?: string; severity: string }) {
  const label = classHint ? CLASS_HINT_LABELS[classHint] : undefined;
  if (!label) {
    return null;
  }
  const toneClass =
    classHint === "advisory"
      ? "border-slate-200 bg-slate-50 text-slate-600"
      : "border-amber-200 bg-amber-50 text-amber-800";
  return (
    <span
      className={`inline-flex shrink-0 rounded border px-1.5 py-0.5 text-[11px] font-semibold ${toneClass}`}
      title={classHint}
    >
      {label}
    </span>
  );
}

function SeverityBadge({ severity }: { severity: string }) {
  const label = severityLabel(severity);
  const toneClass = isRequiredFindingSeverity(severity)
    ? "border-amber-200 bg-amber-50 text-amber-800"
    : "border-slate-200 bg-slate-50 text-slate-600";
  return (
    <span
      className={`inline-flex shrink-0 rounded border px-1.5 py-0.5 text-[11px] font-semibold ${toneClass}`}
      title={severity}
    >
      {label}
    </span>
  );
}

function severityLabel(severity: string) {
  switch (severity) {
    case "blocking":
      return "高 · 阻塞";
    case "must_fix":
      return "高 · 必须修复";
    case "suggestion":
      return "低 · 建议";
    default:
      return severity;
  }
}
