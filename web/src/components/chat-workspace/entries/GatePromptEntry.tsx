import { Check, ChevronRight, FileText } from "lucide-react";
import { useRef, useState } from "react";
import type { ChatEntry } from "../../../state/chat-entries";
import {
  selectGateProjection,
  type GateActionBlockReason,
  type GateProjection,
} from "../../../state/workspace-cockpit-projection";
import {
  GATE_ADOPT_FINDINGS_BUTTON_LABEL,
  GATE_ARCHIVE_BADGE_LABEL,
  GATE_FEEDBACK_OPTIONAL_HINT,
  GATE_FEEDBACK_SUBMITTED_NOTE,
  GATE_REVISION_REVIEWING_NOTE,
  GATE_REVISION_RUNNING_NOTE,
  GATE_TERMINATE_BUTTON_LABEL,
  GATE_TERMINATE_CONFIRM_LABEL,
  GATE_TRIAGE_WHY_COPY,
  gateAdoptFindingsFeedback,
  gateBatchConfirmTitle,
  gateBatchConfirmWhyCopy,
  gateDistanceToPass,
  gateFindingsToggleLabel,
  gateWhyAdvisoryCopy,
  gateWhyRequiredCopy,
  isGateTitleSynonymousCopy,
  gateWhyCompileFailureCopy,
  gateWhyCompileFailureRewriteRunningCopy,
  gateWhyCompileFailureRewriteStoppedCopy,
} from "../../../state/gate-prompt-copy";
import type { WorkspaceWsState } from "../../../state/workspace-ws-store-types";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND } from "../../../state/workspace-chat-rebuild";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { ConfirmTwiceButton } from "../cockpit/ConfirmTwiceButton";
import { GateFeedbackEditor } from "../cockpit/GateFeedbackEditor";
import { ChatEntryContainer } from "../ChatEntryContainer";
import {
  BTN_GHOST_CLASS,
  BTN_PRIMARY_CLASS,
  BTN_SECONDARY_CLASS,
  DISCLOSURE_CHEVRON_CLASS,
  DISCLOSURE_SUMMARY_CLASS,
  GATE_CARD_CLASS,
  GATE_CHIP_CLASS,
  GATE_DIVIDER_CLASS,
  GATE_META_TEXT_CLASS,
  GATE_NESTED_BLOCK_CLASS,
  GATE_SECONDARY_TEXT_CLASS,
  GATE_TITLE_CLASS,
} from "../gate-visual-tokens";
import {
  isActionableBlockingFinding,
  isAdvisoryDisposition,
  isBlockingFinding,
  ReviewFindingGroups,
  ReviewOwnerGuidanceBlock,
  reviewFindingsFromEntry,
  reviewOwnerGuidance,
  type FeedbackSenderWithOptions,
} from "../finding-list";
import {
  GateExclusionStatusBar,
  useGateFindingExclusions,
} from "./gate-finding-exclusion-state";
import {
  gateFindingsCrossRoundDelta,
  gateFindingsDeltaCopy,
} from "../../../state/gate-prompt-copy";
import {
  GATE_CONFIRM_CONSEQUENCE_REJECTED,
  GATE_HUMAN_TRIGGER_LABELS,
  GATE_STALE_COMPLETED_COPY,
  GateNextStepGuidance,
  gateBlockReasonHuman,
  gateNextStepGuidance,
  humanizeGateProse,
  gateCompileFailureAutoRewriteStatusCopy,
  gateConfirmBlockedCompileFailureCopy,
  type GateCompileFailureAutoRewriteInput,
} from "./gate-next-step";
import {
  archiveNoteFromEntry,
  blockReasonFromEntry,
  failureMessageFromEntry,
  gateKindFromEntry,
  gateTriggerFromEntry,
  inlineErrorFromEntry,
  remainingBudgetFromEntry,
  reviewGateFromEntry,
  summaryFromEntry,
  verdictFromEntry,
} from "./gate-card-metadata";

/** F-38：门卡产物行的种类名（与生命周期卡片同一套称呼）。 */
const GATE_ARTIFACT_LABELS: Record<string, string> = {
  work_item_plan: "Work Item Plan",
  story: "Story Spec",
  design: "Design Spec",
};

export function GatePromptEntry({
  entry,
  actions,
  onOpenArtifact,
}: {
  entry: ChatEntry;
  actions?: CockpitActionFacade;
  /** F-38：切到产物视图（cockpit 产物审核页签）；缺省即不渲染产物入口（legacy 页）。 */
  onOpenArtifact?: () => void;
}) {
  const [feedback, setFeedback] = useState("");
  const [feedbackSubmitted, setFeedbackSubmitted] = useState(false);
  const recordGateFeedbackSubmission = useWorkspaceStore(
    (state) => state.recordGateFeedbackSubmission,
  );
  const summary = summaryFromEntry(entry);
  const verdict = verdictFromEntry(entry);
  const reviewGate = reviewGateFromEntry(entry);
  const findings = reviewFindingsFromEntry(entry);
  // REQ-WSC-11（F1 修正）：门卡是 owner 真正「要动手」的地方——只要本轮
  // review 携带人话裁决（问题 + ≥2 个互斥选项），就在技术 findings 折叠区
  // 之前呈现人话区；未携带时整块不出现（不渲染空白块）。
  const ownerGuidance = reviewOwnerGuidance(findings);
  // C2（REQ-HGC-03 场景 1）：阻断口径以 effective class（class_hint 优先，
  // severity 保底）为准——severity=suggestion 而 class_hint=repairable 时
  // 原因行不得再写「不阻断发布」。
  // REQ-WSC-12 §2 前端镜像：只数 actionable——advisory（非本 Work Item 范围
  // 事实）不进「必须处理 N 项」计数，即使 severity/class_hint 判阻断。
  const requiredFindings = findings.filter(isActionableBlockingFinding);
  const requiresTriage = reviewGate === "user_triage_required";
  // F-38：确认者必须知道在确认什么——门卡说明区带出待确认产物版本与入口。
  // 没有产物版本即不渲染（fail-closed 不猜）；版本取 is_current，缺省退最新一轮。
  const workspaceType = useWorkspaceStore((state) => state.workspaceType);
  const pendingArtifactVersion = useWorkspaceStore((state) => {
    const versions = state.artifactVersions ?? [];
    if (versions.length === 0) {
      return null;
    }
    return (
      versions.find((version) => version.is_current === true) ??
      versions.reduce((latest, version) =>
        version.version > latest.version ? version : latest,
      )
    );
  });
  // F-50 裁决 1/3：反馈与确认并行，确认按钮统一「确认当前版本」——与原因行的
  // 「可直接确认」和帮助文案同一称呼。
  const confirmLabel = "确认当前版本";
  // L1（REQ-RET-02）：request-change 按钮随 legacy 决策发送面删除；findings 仅作呈现。
  const isResolved = entry.resolved === true;
  const gateTrigger = gateTriggerFromEntry(entry);
  const remainingBudget = remainingBudgetFromEntry(entry);
  const failureMessage = failureMessageFromEntry(entry);
  const inlineError = inlineErrorFromEntry(entry);
  // 根因 E（REQ-WSC-15 前端面）：validation_reject 失败门的可操作化事实——
  // 被拒反馈原文/可读原因/下一步动作/编译诊断（§6 camelCase 契约；旧失败门
  // 缺席 = 无该块，仍走既有 failure 行，不猜）。
  const rejectionInfo = gateRejectionInfoFromEntry(entry);
  // REQ-WSC-18（owner 2026-10-31 意见 1/2）：当前候选编译失败事实——在场即
  // 「候选编译不过」态：醒目红框（GateCandidateCompileFailureBlock）+ 顶部
  // 指引分支 + 原因行 + 距通过 approve 行 + 确认按钮禁用原因，全部由这同
  // 一份事实派生（owner 0009 现场三处指引互相矛盾的根因修复）。缺席=候选
  // 可编译，零行为变化。
  const candidateCompileFailure = candidateCompileFailureFromEntry(entry);
  const confirmBlockedByCompileFailure = candidateCompileFailure !== null;
  // 禁用原因两阶段取词（running/stopped/无 auto_rewrite），与顶部指引、
  // 红框阶段行同一份事实。
  const confirmBlockedByCompileFailureCopy = gateConfirmBlockedCompileFailureCopy(
    candidateCompileFailure?.autoRewrite ?? null,
  );
  const isContextBlockerGate =
    gateKindFromEntry(entry) === WORK_ITEM_PLAN_CONTEXT_BLOCKER_GATE_KIND;
  const actionFacade =
    (entry.metadata as Record<string, unknown> | undefined)?.action_facade;
  const typedGateAwaitingCommand =
    actionFacade === "typed" &&
    typeof (entry.metadata as Record<string, unknown> | undefined)?.command_id !== "string";
  const persistedActionBlockReason = blockReasonFromEntry(entry, "action_block_reason");
  // F-21：终止判据缺省（undefined）回退通用判据；显式 null=终止放行（?? 会把
  // null 吞成回退，必须辨 undefined）。
  const persistedTerminateBlockReason =
    (entry.metadata as Record<string, unknown> | undefined)?.terminate_block_reason !== undefined
      ? blockReasonFromEntry(entry, "terminate_block_reason")
      : persistedActionBlockReason;
  // F-21：终止与确认共用「活投影匹配 + stage 前缀离场兜底」骨架，仅判据不同
  // （terminate_block_reason 允许 plan 会话 human_confirm 非终审门放行终止）。
  const actionBlockReason = useWorkspaceStore((state) =>
    gateCardBlockReason(state, entry, persistedActionBlockReason, (projection) =>
      projection.action_block_reason ?? null,
    ),
  );
  const terminateBlockReason = useWorkspaceStore((state) =>
    gateCardBlockReason(state, entry, persistedTerminateBlockReason, (projection) =>
      projection.terminate_block_reason !== undefined
        ? projection.terminate_block_reason
        : (projection.action_block_reason ?? null),
    ),
  );
  // owner 2026-10-31 真机（会话 workspace_session_auto_111eafb6…）：计划已确认完成，
  // 对话流里的过期门卡仍写「这扇门不再接收操作…只能终止此门后重新发起」——owner
  // 读成「我卡死了，只能重来」。该卡是历史记录、不是待办门：文案直说「无需处理」。
  // 判据单一来源：durable session_status（confirmed）；只对过期卡（action_block_reason
  // 非空）分流——活门与未完成会话一律沿用既有措辞。
  const sessionCompleted = useWorkspaceStore((state) => state.sessionStatus === "confirmed");
  const staleCompleted = sessionCompleted && actionBlockReason !== null;
  // C2（REQ-HGC-02 场景 1/F-54）：批次确认门专属标题/why——与候选修订门在
  // 标题/原因行层级可区分；确认动作走 confirmBatch（HTTP），无反馈编辑器。
  const isBatchConfirmGate = gateKindFromEntry(entry) === "batch_confirm";
  // C2K3 P3（REQ-HGC-01）：gate-local 已接受反馈轮次——CAS/serde/wire/store
  // 已端到端接线，此处补渲染消费（预算行「已反馈 N 轮 · 剩余 M」）。仅
  // plan 候选修订门当前卡消费；旧会话缺席（undefined/null）= 预算历史
  // 不可用，不猜（不渲染该段）。
  const acceptedFeedbackTurns = useWorkspaceStore((state) =>
    !isBatchConfirmGate && !isResolved && state.workspaceType === "work_item_plan"
      ? (state.humanGateSnapshot?.accepted_feedback_turns ?? null)
      : null,
  );
  // F-50 裁决 1（单标题制）：默认「需要人工确认」，仅 triage intent 门保留
  // 「需要判断 reviewer 意图」——标题回答「现在要做什么」，原因与建议交给
  // gate-why 单一原因行，不再三层同义。
  const title = isBatchConfirmGate
    ? gateBatchConfirmTitle()
    : requiresTriage
      ? "需要判断 reviewer 意图"
      : "需要人工确认";
  // F-50 裁决 2：gate-why = 单一原因行（含下一步建议）。triage 门给 intent
  // 原因行；候选编译不过（REQ-WSC-18）先于 findings 分级——此态原因行绝
  // 不说「可直接确认」，与顶部指引/状态行同一份事实；无 findings 不猜。
  const whyCopy = isBatchConfirmGate
    ? gateBatchConfirmWhyCopy()
    : requiresTriage
      ? GATE_TRIAGE_WHY_COPY
      : confirmBlockedByCompileFailure
        ? candidateCompileFailure?.autoRewrite?.phase === "running"
          ? gateWhyCompileFailureRewriteRunningCopy()
          : candidateCompileFailure?.autoRewrite?.phase === "stopped"
            ? gateWhyCompileFailureRewriteStoppedCopy()
            : gateWhyCompileFailureCopy()
        : findings.length === 0
          ? null
          : requiredFindings.length > 0
            ? gateWhyRequiredCopy(requiredFindings.length)
            : gateWhyAdvisoryCopy(findings.length);
  // C2（REQ-HGC-02 场景 2）：距通过清单——优先用 durable 快照 findings（带
  // class，可判预检缺口），缺席退卡面 findings；仅 plan 候选修订门渲染。
  const snapshotFindings = useWorkspaceStore(
    (state) => state.humanGateSnapshot?.findings ?? null,
  );
  const workspaceTypeForDistance = useWorkspaceStore((state) => state.workspaceType);
  const distanceItems =
    !isBatchConfirmGate &&
    !isResolved &&
    workspaceTypeForDistance === "work_item_plan"
      ? gateDistanceToPass({
          findings: (snapshotFindings ?? findings).map((finding) =>
            "class" in finding && typeof finding.class === "string"
              ? { severity: finding.severity, class: finding.class }
              : { severity: finding.severity },
          ),
          trigger: gateTrigger,
          approveAvailable: actionBlockReason === null,
          // REQ-WSC-18：approve 行与顶部指引/原因行同一份事实——此态不再
          // 显示「可直接确认（或先反馈修订）」。
          candidateCompileFailure: confirmBlockedByCompileFailure,
        })
      : null;
  // C2（REQ-HGC-02 场景 3）：跨轮 delta——current 取 durable 快照 findings
  // （C1 fingerprint+identity_unstable），previous 取 store 保留的上一轮快照
  // （缺席=刷新/历史不全 → 显式 unknown，不猜）；仅 plan 候选修订门渲染。
  const previousGateFindings = useWorkspaceStore(
    (state) => state.previousGateFindings,
  );
  const findingsDelta =
    !isBatchConfirmGate &&
    !isResolved &&
    workspaceTypeForDistance === "work_item_plan" &&
    snapshotFindings !== null
      ? gateFindingsCrossRoundDelta(snapshotFindings, previousGateFindings)
      : null;
  const archiveNote = archiveNoteFromEntry(entry);
  // F-49 B6：adoptable = advisory findings（must_fix 处理路径不同，不进默认采纳）。
  // 采纳按钮的目标是下方反馈输入框——编辑器不可用即无处可填，不露按钮（fail-closed）。
  const adoptableFindings = findings.filter((finding) => !isBlockingFinding(finding));
  const feedbackEditorVisible =
    !isResolved &&
    terminateBlockReason === null &&
    actions !== undefined &&
    actionFacade === "typed" &&
    actionBlockReason === null;
  const showAdoptFindingsButton = adoptableFindings.length > 0 && feedbackEditorVisible;
  // F-49 B4：门内修订进程（提交后「正在按反馈修订」→ 完成后「正在复评」）。只取
  // 「本卡就是当前 typed turn」的门卡（turn_id 匹配活 turn）——门内轮次切换后旧卡
  // 由 A1 收口为留档，不会误报进程。failed 交给失败行，不另报进程。
  const entryTurnId =
    (entry.metadata as Record<string, unknown> | undefined)?.turn_id;
  const liveTurnStatus = useWorkspaceStore((state) =>
    typeof entryTurnId === "string" && state.humanGateTurn?.turn_id === entryTurnId
      ? state.humanGateTurn.status
      : null,
  );
  const revisionProgressCopy =
    liveTurnStatus === "open" || liveTurnStatus === "busy"
      ? GATE_REVISION_RUNNING_NOTE
      : liveTurnStatus === "awaiting_confirm"
        ? GATE_REVISION_REVIEWING_NOTE
        : null;
  // P2（REQ-WSC-15 归因扩展）：本卡的门身份——store 级排除失败事实按它匹配
  //（buildGatePromptEntry 写入的 gate_identity；旧卡/测试夹具缺省回退卡 id）。
  const rawGateIdentity = (entry.metadata as Record<string, unknown> | undefined)
    ?.gate_identity;
  const gateKey = typeof rawGateIdentity === "string" ? rawGateIdentity : entry.id;

  const handleFeedbackSubmit = (value: string) => {
    // REQ-WSC-15 失败兜底归因：记录「本卡最近一次本地门动作」——后续到达的
    // 门内协议错误据此归因（见 exclusionFrameRefused）。P2：同步记到 store——
    // WsOutMessage::Error 无 turn/command 关联字段，error 帧归因只认它。
    latestLocalGateActionRef.current = "feedback";
    useWorkspaceStore.getState().recordGateLocalAction(gateKey, "feedback");
    if (!actions || !actions.feedback(value)) {
      return;
    }
    // F-49 A4：提交成功即清空输入（此前文本留在框内，用户以为未提交），并把该轮
    // 反馈记到卡上——旧轮留档（B5）用它作摘要。
    recordGateFeedbackSubmission(entry.id, value);
    setFeedback("");
    setFeedbackSubmitted(true);
  };

  const handleAdoptFindings = () => {
    // F-49 B6：把 advisory findings 按模板填入下方反馈输入框——只填入不提交
    // （用户可继续编辑）；已有输入时以空格追加，防覆盖用户已写内容（反馈框是
    // 单行 input，value 净化会剥换行，不用换行连接）。F49-B6-fix1：同一
    // findings 的采纳文本是确定性的，草稿已含该段即跳过（防重复点击重复拼接）。
    const adopted = gateAdoptFindingsFeedback(adoptableFindings);
    setFeedback((current) =>
      current.includes(adopted)
        ? current
        : current.trim()
          ? `${current} ${adopted}`
          : adopted,
    );
  };

  // REQ-WSC-13（owner 裁定取舍 2）：owner 的「这不归本 WI」走结构化排除——
  // 门卡每条 finding 一键「排除此项」，身份用平台算的 findingKey（前端不自
  // 算），出站 = HumanGateFeedback 的 excludedFindingIds（排除-only 帧
  // feedback 为空串）。发送成功即本地置已排除（durable 回执由下一轮投影携
  // 带，本地态只作即时反馈）；失败保持可点（fail-closed，不静默丢）。
  // REQ-WSC-13/14（k3 P0-2C）：排除/撤销排除的本地状态单一事实源拆至
  // gate-finding-exclusion-state.tsx（含刷新后 durable 引导与 REST 撤销）。
  const latestLocalGateActionRef = useRef<"exclusion" | "feedback" | null>(null);
  // P2（REQ-WSC-15 归因扩展）：排除-only 帧落盘失败回的是 WsOutMessage::Error
  //（非 protocol_error）——handler 按 store 的「最近本地门动作」归因后落
  // gateExclusionFailure；本门卡按 gateKey 匹配消费（store 级事实，不随门卡
  // upsert/rebuild 丢失），与 inlineError/拒收证据同款撤销乐观「已排除」。
  const gateExclusionFailure = useWorkspaceStore(
    (state) =>
      state.gateExclusionFailure !== null &&
      state.gateExclusionFailure.gateKey === gateKey
        ? state.gateExclusionFailure
        : null,
  );
  const exclusionFrameRefused =
    (rejectionInfo !== null && rejectionInfo.rejectedFeedback.trim() === "") ||
    (inlineError !== null && latestLocalGateActionRef.current === "exclusion") ||
    gateExclusionFailure !== null;
  // §6 契约：门面 feedback 的双参扩展签名；旧门面（单参）天然可赋值，对空串
  // 返回 false → 排除按发送失败呈现，升级后自动走 excludedFindingIds 出站。
  const sendFeedbackWithOptions: FeedbackSenderWithOptions | undefined = actions?.feedback;
  const exclusions = useGateFindingExclusions({
    cardId: entry.id,
    sendFeedbackWithOptions,
    revokeSender: actions?.revokeFindingExclusions,
    exclusionFrameRefused,
    markLocalAction: (kind) => {
      latestLocalGateActionRef.current = kind;
      // P2：归因线索同步入 store（error 帧 handler 侧可读；同时清旧失败事实）。
      useWorkspaceStore.getState().recordGateLocalAction(gateKey, kind);
    },
    // 收口/无带 key finding 的卡不发只读引导 POST（留档卡不逐张打端点）。
    enabled:
      !isResolved && findings.some((finding) => Boolean(finding.findingKey)),
  });
  // 本卡口径的已排除 key：durable 引导带回的清单可能含其他轮次/卡片的 key，
  // 门卡徽章/计数/全部撤销只认本卡 findings 的 key（与本地排除时代同口径）。
  const cardFindingKeys = new Set(
    findings.flatMap((finding) => (finding.findingKey ? [finding.findingKey] : [])),
  );
  const cardExcludedFindingKeys = exclusions.effectiveExcludedFindingKeys.filter((key) =>
    cardFindingKeys.has(key),
  );
  // owner 2026-10-30 真机反馈：门卡最上方给一行「现在请做 X」——按门的状态
  // 分支给出下一步动作（选什么/写什么/点哪个按钮），不再只复述状态。
  const nextStepCopy = gateNextStepGuidance({
    resolved: isResolved,
    rejected: rejectionInfo !== null,
    compileFailure: confirmBlockedByCompileFailure,
    compileFailureAutoRewrite: candidateCompileFailure?.autoRewrite ?? null,
    revisionInProgress: revisionProgressCopy !== null,
    batchConfirm: isBatchConfirmGate,
    contextBlocker: isContextBlockerGate,
    actionBlockReason,
    staleCompleted,
    terminateAvailable: terminateBlockReason === null,
    ownerQuestion: ownerGuidance !== null,
    requiredFindings: requiredFindings.length,
    // k3 瑕疵 ②：归因要准——advisory 只数 disposition=advisory（范围外事实），
    // 建议级 actionable（非阻断 suggestion）单独计 optional，不再用
    // 「总数 − 必处理数」差值把 actionable 冒充 advisory。
    advisoryFindings: findings.filter(isAdvisoryDisposition).length,
    optionalFindings: findings.filter(
      (finding) => !isAdvisoryDisposition(finding) && !isBlockingFinding(finding),
    ).length,
    excludedFindings: cardExcludedFindingKeys.length,
  });
  // 后端投递的 content/summary 可能带机器短语（如「需先处理预判」）——人话化
  // （片段级：已知码映射、黑话只摘除命中片段）后再进同义判定和渲染（见
  // gate-next-step.tsx）。
  const displayContent = humanizeGateProse(entry.content);
  const displaySummary = summary === null ? null : humanizeGateProse(summary);
  return (
    <ChatEntryContainer
      role="system"
      title={title}
      titleSuffix={!isResolved ? <span className={GATE_CHIP_CLASS}>需人工</span> : undefined}
      // F-50 视觉 v2（f50-ui-visual-spec-v2 §1/§2）：门卡改中性白底+4px 琥珀左线
      // +chip——琥珀不再做整卡底色（琥珀压琥珀/零间距连片根因）；类常量与抽屉
      // 门禁条目共用（gate-visual-tokens.ts）。旧 gate-open token 面板退役。
      panelClassName={GATE_CARD_CLASS}
      titleClassName={GATE_TITLE_CLASS}
      testId="gate-prompt-entry"
    >
      {/* F-50 §4.1-3（第一批布局减负）：门卡固定「原因→正文→产物→证据→
          metadata→进度→局部错误→动作」顺序，确认对象（产物）不再被状态行压下去。 */}
      <div className="space-y-3">
        {/* owner 2026-10-30 真机反馈：最上方先给「现在请做 X」，原因/证据随后。 */}
        {nextStepCopy && !isResolved ? (
          // 过期且已完成的门卡没有「现在该做什么」——不套「现在请做：」前缀，
          // 直说无需处理（避免把历史记录读成待办）。
          <GateNextStepGuidance copy={nextStepCopy} prefix={staleCompleted ? "" : undefined} />
        ) : null}
        {whyCopy && !isResolved ? (
          <div
            data-testid="gate-why"
            className={`mt-1 text-sm font-medium ${GATE_SECONDARY_TEXT_CLASS}`}
          >
            {whyCopy}
          </div>
        ) : null}
        {/* F-50 裁决 1：entry.content/summary 与标题同义（人工介入同义句）时
            不渲染——单标题制下不让正文重复标题；独立事实保留（人话化后判定）。 */}
        {isGateTitleSynonymousCopy(displayContent) ? null : (
          <div className="text-sm text-slate-900">{displayContent}</div>
        )}
        {displaySummary &&
        !isGateTitleSynonymousCopy(displaySummary) &&
        displaySummary !== displayContent ? (
          <div className={`text-xs ${GATE_META_TEXT_CLASS}`}>{displaySummary}</div>
        ) : null}
        {!isResolved && pendingArtifactVersion && onOpenArtifact ? (
          <div
            data-testid="gate-artifact-context"
            className={`flex flex-wrap items-center gap-2 text-xs ${GATE_META_TEXT_CLASS}`}
          >
            <span>
              待确认产物：
              {GATE_ARTIFACT_LABELS[workspaceType ?? ""]
                ? `${GATE_ARTIFACT_LABELS[workspaceType ?? ""]} `
                : ""}
              v{pendingArtifactVersion.version}
            </span>
            <button
              type="button"
              data-testid="gate-artifact-open"
              onClick={onOpenArtifact}
              className={BTN_SECONDARY_CLASS}
            >
              <FileText className="h-4 w-4" aria-hidden="true" /> 查看产物
            </button>
          </div>
        ) : null}
        {ownerGuidance ? <ReviewOwnerGuidanceBlock guidance={ownerGuidance} /> : null}
        {findings.length > 0 && !isResolved ? (
          <details
            data-testid="gate-findings"
            className={`group ${GATE_NESTED_BLOCK_CLASS}`}
          >
            <summary
              className={`${DISCLOSURE_SUMMARY_CLASS} flex items-center gap-1 text-xs font-semibold text-slate-900`}
            >
              <ChevronRight className={DISCLOSURE_CHEVRON_CLASS} aria-hidden="true" />
              {gateFindingsToggleLabel(findings.length)}
            </summary>
            <div className="mt-2 space-y-2">
              {showAdoptFindingsButton ? (
                <div className="flex justify-end">
                  <button
                    type="button"
                    data-testid="gate-adopt-findings"
                    onClick={handleAdoptFindings}
                    className={BTN_SECONDARY_CLASS}
                  >
                    {GATE_ADOPT_FINDINGS_BUTTON_LABEL}
                  </button>
                </div>
              ) : null}
              {/* REQ-WSC-13：排除面随反馈编辑器同一放行判据（门面可用且未被
                  阻断）；已排除徽章不依赖发送器——出站成功即保留呈现，但引擎
                  未受理证据在场时按未排除呈现（exclusionFrameRefused）。
                  REQ-WSC-13/14（k3 P0-2C）：已排除行的「撤销排除」走 REST，
                  与门的阻断/收口状态无关（会话级 durable 事实），门面提供即露。 */}
              <ReviewFindingGroups
                findings={findings}
                excludedFindingKeys={new Set(cardExcludedFindingKeys)}
                onExcludeFinding={feedbackEditorVisible ? exclusions.handleExcludeFinding : undefined}
                onRevokeExclusion={
                  actions?.revokeFindingExclusions ? exclusions.handleRevokeExclusion : undefined
                }
                revokingFindingKeys={new Set(exclusions.revokingFindingKeys)}
                revokingAll={exclusions.revokingAll}
              />
              {exclusionFrameRefused && exclusions.excludedFindingKeys.length > 0 ? (
                <p
                  data-testid="gate-finding-exclusion-refused"
                  className="text-xs font-medium text-red-700"
                >
                  刚才的「排除此项」未被引擎受理，以上条目仍为需处理：可重试排除，或在下方写明说明后提交
                </p>
              ) : null}
              {/* REQ-WSC-13/14（k3 P0-2C）：已排除计数注记 + 撤销反馈条
                  （成功即时反馈/失败如实原因/全部撤销）。 */}
              <GateExclusionStatusBar
                excludedCount={cardExcludedFindingKeys.length}
                revokeKeys={cardExcludedFindingKeys}
                revokeAvailable={actions?.revokeFindingExclusions !== undefined}
                revokingAll={
                  exclusions.revokingAll || exclusions.revokingFindingKeys.length > 0
                }
                onRevokeAll={exclusions.handleRevokeAllExclusions}
                failure={exclusions.revokeFailure}
                notice={exclusions.revokeNotice}
              />
            </div>
          </details>
        ) : null}
        {/* C2（REQ-HGC-02 场景 3）：跨轮 delta 计数行——新增/已解决/复现，
            历史不全或身份 unstable 时如实显示 unknown（不猜）。 */}
        {findingsDelta ? (
          <p
            data-testid="gate-findings-delta"
            className={`text-xs font-medium ${GATE_META_TEXT_CLASS}`}
          >
            {gateFindingsDeltaCopy(findingsDelta)}
          </p>
        ) : null}
        {/* F-50 §4.1-4：trigger 与预算合并为一条弱化元数据行（不再两个胶囊
            抢主视觉）。owner 2026-10-30：计数保留、措辞人话——「触发：引擎
            判定需人工 · 已反馈 N 轮 · 剩余修复轮次 M」改为 owner 能懂的说法；
            零轮次/旧会话缺席维持既有形态（缺席段不渲染）。 */}
        {gateTrigger || remainingBudget !== null || (acceptedFeedbackTurns ?? 0) > 0 ? (
          <p data-testid="gate-meta" className={`text-xs ${GATE_META_TEXT_CLASS}`}>
            {[
              gateTrigger
                ? `为什么找你：${GATE_HUMAN_TRIGGER_LABELS[gateTrigger]}`
                : null,
              acceptedFeedbackTurns !== null && acceptedFeedbackTurns > 0
                ? `你已回复过 ${acceptedFeedbackTurns} 轮`
                : null,
              remainingBudget !== null ? `还可自动修复 ${remainingBudget} 次` : null,
            ]
              .filter((part): part is string => Boolean(part))
              .join(" · ")}
          </p>
        ) : null}
        {/* C2（REQ-HGC-02 场景 2/F-52 §三）：「距通过」清单——从 durable
            findings/trigger/阻断判据派生的收敛路径；ok=已满足、pending=待处理、
            unknown=事实未同步（不猜）。 */}
        {distanceItems ? (
          <ul
            data-testid="gate-distance-to-pass"
            className={`space-y-1 text-xs ${GATE_META_TEXT_CLASS}`}
          >
            {distanceItems.map((item) => (
              <li key={item.key} data-testid={`gate-distance-${item.key}`}>
                {item.status === "ok" ? "✓" : item.status === "pending" ? "•" : "?"}{" "}
                {item.label}
              </li>
            ))}
          </ul>
        ) : null}
        {revisionProgressCopy && !isResolved ? (
          <div
            data-testid="gate-revision-status"
            className="text-xs font-medium text-slate-900"
          >
            {revisionProgressCopy}
          </div>
        ) : null}
        {/* F-50 §2.3-7/§2.7：门卡只保留与该门动作直接相关的局部错误（租约/
            连接级 hard error 由页级错误面与收件箱承载），置于动作区之前。 */}
        {failureMessage ? (
          <div
            data-testid="gate-failure"
            className="aria-mono text-xs text-[var(--aria-danger)]"
          >
            {failureMessage}
          </div>
        ) : null}
        {/* REQ-WSC-18：候选编译失败醒目红框——置于拒收块之前（本门的头号
            事实：候选无效需重新生成，确认不放行）；不折叠、不与 findings
            混组（owner 2026-10-31 意见 1）。 */}
        {candidateCompileFailure ? (
          <GateCandidateCompileFailureBlock failure={candidateCompileFailure} />
        ) : null}
        {rejectionInfo ? <GateRejectionBlock info={rejectionInfo} /> : null}
        {inlineError ? (
          <div
            data-testid="gate-inline-protocol-error"
            className="aria-mono text-xs text-[var(--aria-danger)]"
          >
            {inlineError.code} · {inlineError.message}
          </div>
        ) : null}
        {/* P2（REQ-WSC-15 归因扩展）：归因到本门的排除失败——服务端 error 帧
            原文即 owner 可读原因；乐观「已排除」已随 exclusionFrameRefused
            撤销（排除按钮回到可用，可重试）。 */}
        {gateExclusionFailure ? (
          <div
            data-testid="gate-exclusion-failure"
            className="text-xs font-medium text-red-700"
          >
            排除未生效（{gateExclusionFailure.code}）：{gateExclusionFailure.message}
          </div>
        ) : null}
        {isContextBlockerGate && !isResolved ? (
          <div className={`text-xs ${GATE_META_TEXT_CLASS}`}>
            请在下方输入补充上下文后发送，或终止此门
          </div>
        ) : null}
        {isResolved ? (
          <div className="space-y-2">
            {archiveNote ? (
              <div data-testid="gate-archive-note" className={`text-xs ${GATE_META_TEXT_CLASS}`}>
                {archiveNote}
              </div>
            ) : null}
            <ResolutionBadge resolution={entry.resolution} />
          </div>
        ) : terminateBlockReason === null && actions ? (
          // F-21：终止放行即渲染动作位——phase_mismatch 的 plan 门（context
          // blocker/author 失败/缺相位）此前整面消失，终止零通路；现在露出
          // 终止（二次确认惯例），confirm/反馈编辑器维持相位纪律不渲染。
          // F-50 §4.1-5：动作区以分隔线成组，与说明区视觉分层。
          <div className={`space-y-2 ${GATE_DIVIDER_CLASS}`}>
            {/* C2（REQ-HGC-02/plan T2 ④，F-54 §4.2）：phase_mismatch 旧「门相位
                与当前阶段不一致」提示行退役（0009 现场误导）。owner 2026-10-30：
                阻断必须有可见原因——本行给人话原因（phase 用陈述事实的措辞），
                与下方禁用的确认按钮 title 同源。 */}
            {confirmBlockedByCompileFailure && !staleCompleted ? (
              // REQ-WSC-18：候选编译不过——确认禁用的可见原因（含出路），
              // 优先于通用阻断原因（本态是更具体的事实）。
              <p
                data-testid="gate-confirm-blocked-reason"
                className={`text-xs ${GATE_META_TEXT_CLASS}`}
              >
                {confirmBlockedByCompileFailureCopy}
              </p>
            ) : actionBlockReason !== null ? (
              <p
                data-testid="gate-confirm-blocked-reason"
                className={`text-xs ${GATE_META_TEXT_CLASS}`}
              >
                {staleCompleted ? (
                  GATE_STALE_COMPLETED_COPY
                ) : (
                  <>
                    {isContextBlockerGate ? "" : "暂不可确认："}
                    {gateBlockReasonHuman(actionBlockReason)}
                    {terminateBlockReason === null ? "；可终止此门后重新发起" : ""}
                  </>
                )}
              </p>
            ) : null}
            {actionFacade === "typed" && actionBlockReason === null ? (
              <>
                {/* F-50 裁决 3：反馈与确认并行——帮助文案明示反馈可选；修订
                    进行中隐藏静态指导（§2.3-5）。context blocker 门有自己的
                    专属指导，不叠两句。REQ-WSC-18：候选编译不过时也隐藏——
                    「确认当前版本无需填写」在确认已禁用的态失真，反馈出路
                    由顶部指引与红框承载。 */}
                {!revisionProgressCopy &&
                !isContextBlockerGate &&
                !confirmBlockedByCompileFailure ? (
                  <p
                    data-testid="gate-feedback-hint"
                    className={`text-xs ${GATE_META_TEXT_CLASS}`}
                  >
                    {GATE_FEEDBACK_OPTIONAL_HINT}
                  </p>
                ) : null}
                <GateFeedbackEditor
                  multiline={false}
                  value={feedback}
                  onChange={setFeedback}
                  onSubmit={handleFeedbackSubmit}
                />
              </>
            ) : null}
            {feedbackSubmitted && !revisionProgressCopy ? (
              <p
                data-testid="gate-feedback-submitted"
                className="text-xs font-medium text-emerald-700"
              >
                {GATE_FEEDBACK_SUBMITTED_NOTE}
              </p>
            ) : null}
            {typedGateAwaitingCommand && actionBlockReason === null ? (
              <p className={`text-xs ${GATE_META_TEXT_CLASS}`}>
                未同步门命令，将以新命令提交
              </p>
            ) : null}
            <div className="flex flex-wrap items-center justify-end gap-2">
              {/* owner 2026-10-30 反馈（不知道怎么继续）：确认按钮不再无声消失——
                  阻断时以禁用态留在卡上（title 与上方原因行给人话原因）；反馈
                  被拒时它是明确出路，保持可点并给后果说明。 */}
              {isContextBlockerGate ? null : (
                // C2（REQ-HGC-02）：批次门确认=HTTP confirmBatch（发布语义）；
                // 候选修订门维持 WS confirm（确认当前版本）。
                <button
                  type="button"
                  data-testid="gate-confirm-button"
                  disabled={actionBlockReason !== null || confirmBlockedByCompileFailure}
                  title={
                    staleCompleted
                      ? // 过期且已完成：直说无需处理（不指向终止/重新发起）。
                        GATE_STALE_COMPLETED_COPY
                      : confirmBlockedByCompileFailure
                        ? // REQ-WSC-18：平台侧判据（metadata candidate_compile_failure
                          // 在场即该态）——fail-closed：确认此候选只会再次编译失败。
                          confirmBlockedByCompileFailureCopy
                        : actionBlockReason !== null
                          ? gateBlockReasonHuman(actionBlockReason)
                          : rejectionInfo && !isBatchConfirmGate
                            ? GATE_CONFIRM_CONSEQUENCE_REJECTED
                            : undefined
                  }
                  onClick={() => {
                    if (isBatchConfirmGate) {
                      void actions.confirmBatch();
                    } else {
                      actions.confirm();
                    }
                  }}
                  className={`${BTN_PRIMARY_CLASS} disabled:pointer-events-none disabled:opacity-50`}
                >
                  <Check className="h-4 w-4" aria-hidden="true" />
                  {isBatchConfirmGate ? gateBatchConfirmTitle() : confirmLabel}
                </button>
              )}
              {rejectionInfo &&
              actionBlockReason === null &&
              !isBatchConfirmGate &&
              !confirmBlockedByCompileFailure ? (
                <span
                  data-testid="gate-confirm-consequence"
                  className={`text-xs ${GATE_META_TEXT_CLASS}`}
                >
                  {GATE_CONFIRM_CONSEQUENCE_REJECTED}
                </span>
              ) : null}
              {/* 过期且已完成的门卡是历史记录——不挂终止按钮（终态会话终止
                  无意义、服务端也会拒；留着就是假按钮）。 */}
              {staleCompleted ? null : (
                <ConfirmTwiceButton
                  variant="ghost"
                  label={GATE_TERMINATE_BUTTON_LABEL}
                  confirmLabel={GATE_TERMINATE_CONFIRM_LABEL}
                  onConfirm={actions.terminate}
                />
              )}
            </div>
          </div>
        ) : actions ? (
          // 终止亦被阻断（典型：阶段离开后的残留卡）——此前动作面整块消失，
          // owner 面对「无声死结」。现在确认按钮以禁用态+原因留在卡上；终止
          // 不可用时不渲染。actions 缺席（legacy 只读页）维持零按钮。
          <div className={`space-y-2 ${GATE_DIVIDER_CLASS}`}>
            {actionBlockReason !== null ? (
              <p
                data-testid="gate-confirm-blocked-reason"
                className={`text-xs ${GATE_META_TEXT_CLASS}`}
              >
                {staleCompleted
                  ? GATE_STALE_COMPLETED_COPY
                  : `暂不可确认：${gateBlockReasonHuman(actionBlockReason)}`}
              </p>
            ) : confirmBlockedByCompileFailure ? (
              // REQ-WSC-18：此态不得说「仍可确认当前版本继续」——与主分支
              // 同一份事实/同一句禁用原因。
              <p
                data-testid="gate-confirm-blocked-reason"
                className={`text-xs ${GATE_META_TEXT_CLASS}`}
              >
                {confirmBlockedByCompileFailureCopy}
              </p>
            ) : isContextBlockerGate ? (
              <p className={`text-xs ${GATE_META_TEXT_CLASS}`}>
                终止暂不可用；请以页面最新状态为准
              </p>
            ) : (
              <p className={`text-xs ${GATE_META_TEXT_CLASS}`}>
                终止暂不可用；仍可确认当前版本继续
              </p>
            )}
            <div className="flex flex-wrap items-center justify-end gap-2">
              {isContextBlockerGate ? null : (
                <button
                  type="button"
                  data-testid="gate-confirm-button"
                  disabled={actionBlockReason !== null || confirmBlockedByCompileFailure}
                  title={
                    staleCompleted
                      ? GATE_STALE_COMPLETED_COPY
                      : confirmBlockedByCompileFailure
                        ? confirmBlockedByCompileFailureCopy
                        : actionBlockReason !== null
                          ? gateBlockReasonHuman(actionBlockReason)
                          : GATE_CONFIRM_CONSEQUENCE_REJECTED
                  }
                  onClick={() => {
                    if (isBatchConfirmGate) {
                      void actions.confirmBatch();
                    } else {
                      actions.confirm();
                    }
                  }}
                  className={`${BTN_PRIMARY_CLASS} disabled:pointer-events-none disabled:opacity-50`}
                >
                  <Check className="h-4 w-4" aria-hidden="true" />
                  {isBatchConfirmGate ? gateBatchConfirmTitle() : confirmLabel}
                </button>
              )}
            </div>
          </div>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}

function ResolutionBadge({ resolution }: { resolution?: string }) {
  if (resolution === "confirm") {
    return (
      <span className="inline-flex items-center rounded-md bg-emerald-50 px-2 py-1 text-xs font-semibold text-emerald-700 ring-1 ring-emerald-200">
        已确认
      </span>
    );
  }
  if (resolution === "request-change") {
    return (
      <span className="inline-flex items-center rounded-md bg-amber-50 px-2 py-1 text-xs font-semibold text-amber-700 ring-1 ring-amber-200">
        已要求修改
      </span>
    );
  }
  if (resolution === "superseded") {
    return (
      <span
        data-testid="gate-archive-badge"
        className="inline-flex items-center rounded-md bg-slate-100 px-2 py-1 text-xs font-semibold text-slate-600 ring-1 ring-slate-200"
      >
        {GATE_ARCHIVE_BADGE_LABEL}
      </span>
    );
  }
  if (resolution === "terminate") {
    return (
      <span className="inline-flex items-center rounded-md bg-red-50 px-2 py-1 text-xs font-semibold text-red-700 ring-1 ring-red-200">
        已终止
      </span>
    );
  }
  return null;
}

// 通用 stage 前缀门离场兜底（k3 P2-1）：门卡不随 stage_change 重建（仅
// setStage），阶段离开后投影消失——按 gateIdentity 与当前 stage 不一致判
// terminal_stage，避免重渲染出可点但被静默拦截的假按钮（human_confirm 与
// F-20 story/design author_confirm 门同款纪律）。
//
// F-49 A2：卡身份与当前门投影不一致即为「该卡不是当前门」——门内轮次切换时门载体
// 从 durable snapshot 切到 typed turn，旧卡 id 随之变化并经 upsert 留档；任何未
// 留档的残留卡都不得回退 persistedReason（= 开门时刻的 action_block_reason，实测
// null）放行动作：页面只挂一个动作门面，点旧卡实际作用于当前门（诊断 §0-B）。
function gateCardBlockReason(
  state: WorkspaceWsState,
  entry: ChatEntry,
  persistedReason: Exclude<GateActionBlockReason, null> | null,
  fromProjection: (projection: GateProjection) => Exclude<GateActionBlockReason, null> | null,
): Exclude<GateActionBlockReason, null> | null {
  const gateIdentity = (entry.metadata as Record<string, unknown> | undefined)?.gate_identity;
  if (typeof gateIdentity !== "string") {
    return persistedReason;
  }
  const projection = selectGateProjection(state);
  if (projection?.key === gateIdentity) {
    return projection.turn && state.stage !== "human_confirm"
      ? "terminal_stage"
      : fromProjection(projection);
  }
  // stage 前缀门（legacy story/design author_confirm）：身份即阶段，阶段变了才是
  // 离场；其余（typed turn / durable snapshot 载体）身份不匹配即被取代 → 已关闭。
  if (gateIdentity.startsWith("stage:")) {
    return gateIdentity === `stage:${state.stage}` ? persistedReason : "terminal_stage";
  }
  return "closed";
}
/** 根因 E：validation_reject 可操作化呈现所需的最小事实（全部可缺席）。 */
type GateRejectionInfo = {
  rejectedFeedback: string;
  rejectionReason: string;
  rejectionNextActions: string[];
  rejectionDiagnostics: string[];
};

function gateRejectionInfoFromEntry(entry: ChatEntry): GateRejectionInfo | null {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const rejectedFeedback =
    typeof metadata?.rejectedFeedback === "string" ? metadata.rejectedFeedback : null;
  // owner 2026-10-30：被拒原因是 REQ-WSC-14「可读原因」——已知机器码映射，
  // 未解释术语逐字保留（jargon: "keep"，不套黑话兜底：整段/片段改写都会
  // 丢真实原因）；下一步动作仍按默认片段摘除；rejectedFeedback 是 owner
  // 自己的话、diagnostics 是逐字诊断，均保持原文。
  const rejectionReason =
    typeof metadata?.rejectionReason === "string"
      ? humanizeGateProse(metadata.rejectionReason, { jargon: "keep" })
      : null;
  const rejectionNextActions = Array.isArray(metadata?.rejectionNextActions)
    ? metadata.rejectionNextActions
        .filter((item): item is string => typeof item === "string")
        .map((item) => humanizeGateProse(item))
    : [];
  const rejectionDiagnostics = Array.isArray(metadata?.rejectionDiagnostics)
    ? metadata.rejectionDiagnostics.filter((item): item is string => typeof item === "string")
    : [];
  if (
    !rejectedFeedback &&
    !rejectionReason &&
    rejectionNextActions.length === 0 &&
    rejectionDiagnostics.length === 0
  ) {
    return null;
  }
  return {
    rejectedFeedback: rejectedFeedback ?? "",
    rejectionReason: rejectionReason ?? "",
    rejectionNextActions,
    rejectionDiagnostics,
  };
}

/** REQ-WSC-18：候选编译失败的可呈现事实（缺省字段显式 null/[]，不猜）。 */
type GateCandidateCompileFailureInfo = {
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
function candidateCompileFailureFromEntry(
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
function GateCandidateCompileFailureBlock({
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

/**
 * 根因 E（REQ-WSC-15 前端面）：validation_reject 门 turn 的可操作化呈现——
 * 此前失败门只有 `failure_class · failure_message` 一行，owner 不知道被拒的是
 * 哪条反馈、为什么、下一步做什么（现场卡死主因）。被拒反馈与原因可读呈现，
 * 诊断逐字（不改写、不截断）。
 */
function GateRejectionBlock({ info }: { info: GateRejectionInfo }) {
  return (
    <section
      data-testid="gate-rejection-block"
      className="space-y-2 rounded-md border border-red-200 bg-red-50 px-3 py-2"
    >
      <div className="text-xs font-semibold text-red-800">上一条反馈被拒收</div>
      {info.rejectedFeedback ? (
        <div data-testid="gate-rejected-feedback" className="text-sm text-slate-900">
          被拒反馈：{info.rejectedFeedback}
        </div>
      ) : null}
      {info.rejectionReason ? (
        <div data-testid="gate-rejection-reason" className="text-sm text-slate-900">
          原因：{info.rejectionReason}
        </div>
      ) : null}
      {info.rejectionNextActions.length > 0 ? (
        <ul
          data-testid="gate-rejection-next-actions"
          className="list-disc space-y-1 pl-5 text-sm text-slate-900"
        >
          {info.rejectionNextActions.map((action, index) => (
            <li key={`${action}-${index}`}>{action}</li>
          ))}
        </ul>
      ) : null}
      {info.rejectionDiagnostics.length > 0 ? (
        <pre
          data-testid="gate-rejection-diagnostics"
          className="aria-mono overflow-x-auto whitespace-pre-wrap text-xs text-[var(--aria-ink)]"
        >
          {info.rejectionDiagnostics.join("\n")}
        </pre>
      ) : null}
    </section>
  );
}
