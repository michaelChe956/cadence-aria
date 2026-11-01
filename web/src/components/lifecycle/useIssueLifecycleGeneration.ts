import { useRef, type Dispatch, type SetStateAction } from "react";
import {
  generateDesignSpecs,
  generateStorySpecs,
  getAutomationEnrollment,
  getAutomationTarget,
  prepareWorkItemPlan,
  putAutomationEnrollment,
  ApiRequestError,
} from "../../api/client";
import type { IssueAutomationEnrollment, ProductIssue } from "../../api/types";
import type {
  LifecycleCard,
  LifecycleColumns,
} from "../../state/lifecycle-workbench-store";
import {
  reenableOptionsWithProviderChoice,
  sameProviderChoice,
  type ReenableOriginalAuthorization,
} from "./automation-reenable";
import { readWorkspaceProviderDefaultsSnapshot } from "../../state/workspace-provider-defaults";
import {
  defaultLaunchTitle,
  errorMessage,
  lifecycleEntityKey,
} from "./IssueLifecycleWorkbenchParts";
import type { WorkbenchStageKey } from "./StageStepper";
import type {
  AutomationMode,
  WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";

export type ProviderWorkspaceLaunchTarget = "story" | "design" | "work_item";

export type PendingWorkItemPlanLaunch = {
  card: LifecycleCard;
  /** REQ-PPS-01：开启弹窗即快照表单初值（含用户默认 provider），弹窗期间不被并发写入改动。 */
  options: WorkItemPlanOptionsFormValue;
  /**
   * P1 WIGA：一次用户选择的稳定键——弹窗开启时生成，本次提交重试间不变；
   * 服务端以同键同 payload 幂等、异 payload Conflict 裁决。
   */
  selectionKey: string;
};

const DEFAULT_WORK_ITEM_PLAN_OPTIONS = {
  include_integration_tests: true,
  include_e2e_tests: false,
  force_frontend_backend_split: true,
  require_execution_plan_confirm: false,
  automation_mode: "manual",
} satisfies WorkItemPlanOptionsFormValue;

// REQ-PPS-01 场景三：创建被服务端 fail-closed 拒绝（provider 不可用等 4xx）时的兜底文案，
// 服务端 message 存在时以它为准。
const LAUNCH_FAILURE_MESSAGES: Record<ProviderWorkspaceLaunchTarget, string> = {
  story: "生成 Story Spec 失败",
  design: "生成 Design Spec 失败",
  work_item: "生成 Work Item Plan 失败",
};

const AUTOMATION_ENROLL_FAILURE_MESSAGE = "启用自动化失败";

// 批 2 T4（REQ-WIGA-09 修订）：终止收口未完成时不可重新开启——服务端回
// 409 automation_plan_cleanup_required；该错误有明确的下一步（先重试终止
// 完成清理），不降级为通用冲突文案。
// 重新开启（步骤① Enable）的服务端冲突码：旧授权 source 仍有效但与当前
// 不同、或终止记录被并发修改——有明确下一步（走换代/重绑 rebind），不降级
// 为通用冲突文案。
const AUTOMATION_CLEANUP_REQUIRED_CODE = "automation_plan_cleanup_required";
const AUTOMATION_CLEANUP_REQUIRED_MESSAGE =
  "自动化的终止收口尚未完成：请先重试终止完成清理，再重新开启自动化";
const AUTOMATION_CONFLICT_CODE = "automation_enrollment_conflict";

const AUTOMATION_REENABLE_CONFLICT_MESSAGE =
  "重新开启的自动化授权与终止记录不一致（原授权源仍有效但与当前不同）：请先通过自动化控制面执行「换代 / 重新绑定（rebind）」更新授权源，或恢复原授权源后再重新开启";

export interface IssueLifecycleGenerationOptions {
  selectedProjectId: string | null;
  selectedColumns: LifecycleColumns;
  pendingWorkItemPlanLaunch: PendingWorkItemPlanLaunch | null;
  setPendingWorkItemPlanLaunch: Dispatch<
    SetStateAction<PendingWorkItemPlanLaunch | null>
  >;
  setError: Dispatch<SetStateAction<string | null>>;
  setSelectedCardKey: Dispatch<SetStateAction<string | null>>;
  refresh: (
    projectIdOverride?: string | null,
    optimisticIssues?: readonly ProductIssue[],
  ) => Promise<void>;
  openDrawer: (entityKey: string) => void;
  onOpenWorkspace: (sessionId: string) => void;
}

export interface IssueLifecycleGenerationActions {
  handleLaunchWorkspace: (
    target: ProviderWorkspaceLaunchTarget,
    card: LifecycleCard,
  ) => Promise<void>;
  handleGenerateNext: (card: LifecycleCard) => Promise<void>;
  handleGenerateForStage: (stage: WorkbenchStageKey) => void;
  handleConfirmWorkItemPlanOptions: (
    options: WorkItemPlanOptionsFormValue,
  ) => Promise<void>;
}

/** 随机标识：弹窗实例的 selection_key 与用户动作的 command_id 共用同一生成器。 */
function newRandomId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `id-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

/**
 * Workbench 的 plan/story/design 创建入口（原 IssueLifecycleWorkbench 内联处理器整块抽出，
 * 纯搬运零行为变化）：三处创建请求都带用户默认 provider 快照（REQ-PPS-01），失败统一落到
 * 工作台错误横幅（这些调用点多为 `void xxx(...)`，没有宿主接住 rejection）。
 */
export function useIssueLifecycleGeneration({
  selectedProjectId,
  selectedColumns,
  pendingWorkItemPlanLaunch,
  setPendingWorkItemPlanLaunch,
  setError,
  setSelectedCardKey,
  refresh,
  openDrawer,
  onOpenWorkspace,
}: IssueLifecycleGenerationOptions): IssueLifecycleGenerationActions {
  // 重新开启（terminated enrollment 上的人工 Enable）的命令身份：一次
  // 「重新开启」是新的显式命令，命令身份必须与被终止记录那次 Enable 不同，
  // 否则服务端命令账本无法把它单独留痕（可能被判成旧命令重放而静默不生效）。
  // 同一弹窗实例内重试复用同一身份（幂等），弹窗重开即换新。
  const reenableSelectionKeyRef = useRef<{
    dialogKey: string;
    value: string;
  } | null>(null);

  function reenableSelectionKey(dialogSelectionKey: string): string {
    const current = reenableSelectionKeyRef.current;
    if (current && current.dialogKey === dialogSelectionKey) {
      return current.value;
    }
    const value = newRandomId();
    reenableSelectionKeyRef.current = {
      dialogKey: dialogSelectionKey,
      value,
    };
    return value;
  }

  // 稳定 command_id（REQ-WIGA-09 修订）：同一次用户动作（同一 payload）的
  // 重试必须复用同一个 command_id，服务端据此幂等重放首次 durable 结果；
  // 载荷变了就是另一次动作，换新 command_id（否则同 command_id 配异
  // payload 会被服务端判 409）。
  // 按 payload 摘要分别记账：一次用户动作可能发多条命令（重新开启序列的
  // Enable→Disable→Enable），每条命令各自的 payload 摘要对应各自的稳定 id，
  // 互不覆盖（否则后一条命令会把前一条的重试身份挤掉）。
  const enableCommandIdRef = useRef<Map<string, string>>(new Map());

  function enableCommandId(payloadDigest: string): string {
    const cache = enableCommandIdRef.current;
    const existing = cache.get(payloadDigest);
    if (existing) {
      return existing;
    }
    const commandId = newRandomId();
    cache.set(payloadDigest, commandId);
    return commandId;
  }

  // 重新开启序列（terminated → 换 provider）的身份：一次用户动作可能发多条
  // 命令（Enable 等 payload → Disable → Enable 新 payload）。序列跑到一半
  // 失败后 enrolment 已非 terminated，靠 enrollment_id 认出「同一次重新开启
  // 的后续步骤」，从服务端现状续跑；enrollment 换了/被删则序列作废。
  const reenableSequenceRef = useRef<{
    enrollmentId: string;
    original: ReenableOriginalAuthorization;
  } | null>(null);

  function openWorkItemPlanOptions(card: LifecycleCard) {
    setPendingWorkItemPlanLaunch({
      card,
      options: {
        ...DEFAULT_WORK_ITEM_PLAN_OPTIONS,
        ...readWorkspaceProviderDefaultsSnapshot(),
        automation_mode: "manual" as AutomationMode,
      },
      selectionKey: newRandomId(),
    });
  }

  async function handleGenerateNext(card: LifecycleCard) {
    if (!selectedProjectId) {
      setError("缺少 Project 或生命周期实体");
      return;
    }

    try {
      if (card.kind === "story_spec") {
        const response = await generateDesignSpecs(
          selectedProjectId,
          card.issueId,
          {
            title: defaultLaunchTitle({ target: "design", card }),
            story_spec_ids: [card.id],
            ...readWorkspaceProviderDefaultsSnapshot(),
          },
        );
        const nextId = response.design_specs[0]?.design_spec_id;
        await refresh(selectedProjectId);
        if (nextId) {
          const nextKey = lifecycleEntityKey("design_spec", card.issueId, nextId);
          setSelectedCardKey(nextKey);
          openDrawer(nextKey);
        }
        return;
      }

      if (card.kind === "design_spec") {
        setError(null);
        openWorkItemPlanOptions(card);
        return;
      }

      setError("当前实体不支持生成下一阶段");
    } catch (reason) {
      // 调用方是 `void handleGenerateNext(...)`：与 handleLaunchWorkspace 同款，失败
      // 必须显式落到工作台错误横幅（REQ-PPS-01 场景三）。
      setError(
        errorMessage(
          reason,
          card.kind === "story_spec"
            ? LAUNCH_FAILURE_MESSAGES.design
            : LAUNCH_FAILURE_MESSAGES.work_item,
        ),
      );
    }
  }

  // Task 6：阶段工作区空阶段主按钮接线——复用现有生成链路，不新增 API：
  // story -> 用当前 Issue 卡走 handleLaunchWorkspace("story")；
  // design -> 用最新 Story 卡走 handleGenerateNext（内部走 generateDesignSpecs）；
  // work_item -> 用最新 Design 卡走 handleGenerateNext（打开 Work Item Plan 配置弹窗）。
  function handleGenerateForStage(stage: WorkbenchStageKey) {
    if (stage === "story") {
      const issueCard = selectedColumns.issue[0];
      if (!issueCard) {
        setError("缺少 Issue");
        return;
      }
      void handleLaunchWorkspace("story", issueCard);
      return;
    }

    const sourceCard =
      stage === "design"
        ? selectedColumns.story_spec.at(-1)
        : selectedColumns.design_spec.at(-1);
    if (!sourceCard) {
      setError(stage === "design" ? "缺少 Story Spec" : "缺少 Design Spec");
      return;
    }
    void handleGenerateNext(sourceCard);
  }

  async function handleLaunchWorkspace(
    target: ProviderWorkspaceLaunchTarget,
    card: LifecycleCard,
  ) {
    if (!selectedProjectId) {
      setError("缺少 Project 或生命周期卡片");
      return;
    }

    try {
      if (target === "story") {
        const response = await generateStorySpecs(
          selectedProjectId,
          card.issueId,
          {
            title: defaultLaunchTitle({ target, card }),
            ...readWorkspaceProviderDefaultsSnapshot(),
          },
        );
        const storySpecId = response.story_specs[0]?.story_spec_id;
        setSelectedCardKey(
          storySpecId
            ? lifecycleEntityKey("story_spec", card.issueId, storySpecId)
            : null,
        );
        await refresh(selectedProjectId);
        if (response.workspace_session) {
          onOpenWorkspace(response.workspace_session.workspace_session_id);
        }
        return;
      }

      if (target === "design" && card.kind === "story_spec") {
        const response = await generateDesignSpecs(
          selectedProjectId,
          card.issueId,
          {
            title: defaultLaunchTitle({ target, card }),
            story_spec_ids: [card.id],
            ...readWorkspaceProviderDefaultsSnapshot(),
          },
        );
        const designSpecId = response.design_specs[0]?.design_spec_id;
        setSelectedCardKey(
          designSpecId
            ? lifecycleEntityKey("design_spec", card.issueId, designSpecId)
            : null,
        );
        await refresh(selectedProjectId);
        if (response.workspace_session) {
          onOpenWorkspace(response.workspace_session.workspace_session_id);
        }
        return;
      }

      if (target === "work_item" && card.kind === "design_spec") {
        setError(null);
        openWorkItemPlanOptions(card);
        return;
      }

      setError("当前卡片不能启动该 Workspace");
    } catch (reason) {
      // 调用方是 `void handleLaunchWorkspace(...)`：没有宿主接住这条链路，失败必须
      // 落到工作台错误横幅，否则 provider 不可用等 4xx 对用户完全不可见。
      setError(errorMessage(reason, LAUNCH_FAILURE_MESSAGES[target]));
    }
  }

  /**
   * P1 WIGA：Design 确认后的显式自动化 enrollment。provider/options 一律取
   * 服务端只读 target 投影的已解析值（前端不猜默认）；Story/Design 引用取
   * 当前已确认版本的精确 id+version。相同 selection_key 的失败重试传 GET
   * 到的现行 revision；异 payload 409 由弹窗展示，不自动 Disable/重开。
 * 批 2 T4（REQ-WIGA-09 修订 + 悬空源重授权）：terminated 记录上的人工
 * Enable 是「重新开启」——命令身份换新；source 取当前 card（原 source
 * 悬空时服务端按重授权放行换代留痕），options/target 原样回传终止记录；
 * 用户本次选择的 provider 走契约指定的「先重新开启、再走既有
 * Disable→Enable 路径」落地（见 runReenableSequence）。
   */
  async function enrollConfirmedDesignAutomation(
    projectId: string,
    card: Extract<LifecycleCard, { kind: "design_spec" }>,
    selectionKey: string,
    options: WorkItemPlanOptionsFormValue,
  ) {
    const enrollment = await getAutomationEnrollment(projectId, card.issueId);
    // 批 2 T4（REQ-WIGA-09 修订）：terminated 的 enrollment 可被人工显式
    // 重新开启——入口（人工点「启用自动化」）即显式动作，自动路径不触发。
    // 并发语义不变：expected_revision 按线性化带当前 revision；重新开启的
    // source 取当前 card、options/target 回传终止记录（服务端比对：旧
    // source 仍有效但不同 → 409 提示换代/重绑）；命令身份
    // （command_id/selection_key）换新——重新开启是新命令，须能被命令账本
    // 单独留痕。
    //
    // 重新开启序列的中断态：序列跑到一半失败时 enrolment 已非 terminated，
    // 靠序列身份（enrollment_id）认出「这是同一次重新开启的后续步骤」，
    // 从服务端现状续跑，不重复取代、不留半开状态。
    const sequence = reenableSequenceRef.current;
    const resumingSequence =
      sequence !== null &&
      enrollment !== null &&
      enrollment.enrollment_id === sequence.enrollmentId &&
      enrollment.terminated_at == null;
    if (enrollment?.terminated_at != null || resumingSequence) {
      await runReenableSequence(
        projectId,
        card,
        selectionKey,
        options,
        enrollment,
        sequence,
      );
      return;
    }
    // 序列身份已失效（enrollment 换了/被删）→ 丢弃旧序列状态，走常规路径。
    if (sequence !== null) {
      reenableSequenceRef.current = null;
    }

    const target = await getAutomationTarget(projectId, card.issueId, {
      author_provider: options.author_provider,
      reviewer_provider: options.reviewer_provider,
      include_integration_tests: options.include_integration_tests,
      include_e2e_tests: options.include_e2e_tests,
      force_frontend_backend_split: options.force_frontend_backend_split,
      require_execution_plan_confirm: options.require_execution_plan_confirm,
    });

    const stories = card.raw.story_spec_ids
      .map((storyId) => selectedColumns.story_spec.find((story) => story.id === storyId))
      .filter((story): story is Extract<LifecycleCard, { kind: "story_spec" }> =>
        story?.kind === "story_spec" && story.raw.confirmation_status === "confirmed",
      );
    if (stories.length !== card.raw.story_spec_ids.length) {
      throw new Error("自动化要求引用的 Story Spec 已全部确认");
    }
    if (card.raw.confirmation_status !== "confirmed") {
      throw new Error("自动化要求 Design Spec 已确认");
    }

    const command = {
      type: "enable" as const,
      selection_key: selectionKey,
      source: {
        stories: stories.map((story) => ({
          id: story.id,
          version: story.raw.current_version ?? 1,
        })),
        designs: [
          {
            id: card.id,
            version: card.raw.current_version ?? 1,
          },
        ],
      },
      // B2-P1（REQ-WIGA-02）：自动化入口显式提交 auto——GET 投影与
      // prepare 同源解析、plan_approval 恒为缺省 manual，Enable 的冻结
      // 位由入口选择覆盖；F2 修复：require_execution_plan_confirm 显式
      // 压 false（与后端 normalized_for_enrollment_freeze 同值），wire
      // 与冻结值一致，投影解析的旧表单值不上 wire；手动链路不经本函数。
      options: {
        ...target.resolved_options,
        plan_approval: "auto" as const,
        plan_options: {
          ...target.resolved_options.plan_options,
          require_execution_plan_confirm: false,
        },
      },
      // C5 Task 1：Enable 只回传服务端投影的 enrollment target（必带、
      // 不再发送 logical_repository_id 冗余字段，不从前端拼凑）。
      target: target.enrollment_target,
    };
    await putAutomationEnrollment(projectId, card.issueId, {
      expected_revision: enrollment ? enrollment.policy_revision : null,
      command: {
        ...command,
        command_id: enableCommandId(JSON.stringify(command)),
      },
    });
  }

  /**
   * terminated 记录的人工重新开启（REQ-WIGA-09 修订 + 悬空源重授权）：
   *   ① Enable——source 取**当前 card** 的已确认引用（与常规启用同口径；
   *      终止记录的原 source 可能已随实体删除悬空，服务端按悬空重授权
   *      放行 source 漂移并 binding 换代留痕），options/target 原样回传
   *      终止记录（服务端逐字段比对：旧 source 仍有效但不同 → 409，提示
   *      走「换代/重绑（rebind）」）；
   *   ② 用户本次换了 provider 时：Disable → Enable（新 payload）——契约指定
   *      的「改授权 payload 须先成功重新开启，再走既有 Disable→Enable 路径」；
   *      disabled 记录换 payload 重开是既有语义（服务端放行）。
   *   ③ 每步带稳定 command_id 与 expected_revision；中途失败后重试按服务端
   *      现状续跑（已生效的不重复改写），不重复取代、不留半开状态。
   *
   * 未换 provider 时只发 ①——不产生多余的 Disable/Enable 序列。
   */
  async function runReenableSequence(
    projectId: string,
    card: Extract<LifecycleCard, { kind: "design_spec" }>,
    selectionKey: string,
    formOptions: WorkItemPlanOptionsFormValue,
    enrollment: IssueAutomationEnrollment,
    sequence: { enrollmentId: string; original: ReenableOriginalAuthorization } | null,
  ): Promise<void> {
    // 重新开启的 source＝当前 card 的已确认 Story/Design 引用（与常规启用
    // 路径同一口径）：终止记录的原 source 可能已随级联删除悬空，照抄会让
    // 服务端 422「not found」死路；仍有效但与当前不同时由服务端 409 裁决。
    const stories = card.raw.story_spec_ids
      .map((storyId) => selectedColumns.story_spec.find((story) => story.id === storyId))
      .filter((story): story is Extract<LifecycleCard, { kind: "story_spec" }> =>
        story?.kind === "story_spec" && story.raw.confirmation_status === "confirmed",
      );
    if (stories.length !== card.raw.story_spec_ids.length) {
      throw new Error("自动化要求引用的 Story Spec 已全部确认");
    }
    if (card.raw.confirmation_status !== "confirmed") {
      throw new Error("自动化要求 Design Spec 已确认");
    }
    // 原授权＝当前 source + 终止记录的 options/target（授权选项/载体不被
    // 表单顺便改写）；续跑时沿用序列里记下的原授权（服务端现状已是它的投影）。
    const original: ReenableOriginalAuthorization = sequence?.original ?? {
      source: {
        stories: stories.map((story) => ({
          id: story.id,
          version: story.raw.current_version ?? 1,
        })),
        designs: [
          {
            id: card.id,
            version: card.raw.current_version ?? 1,
          },
        ],
      },
      options: enrollment.options,
      // 旧代 enrollment 可能没有声明 target（无绑定事实）——此时退回
      // 服务端投影的原样 target，由服务端判定是否接受。
      target:
        enrollment.target ??
        (await getAutomationTarget(projectId, card.issueId, {
          author_provider: formOptions.author_provider,
          reviewer_provider: formOptions.reviewer_provider,
          include_integration_tests: formOptions.include_integration_tests,
          include_e2e_tests: formOptions.include_e2e_tests,
          force_frontend_backend_split: formOptions.force_frontend_backend_split,
          require_execution_plan_confirm:
            formOptions.require_execution_plan_confirm,
        })).enrollment_target,
    };
    // 目标授权＝原授权 + 用户本次选择的 provider（其余授权字段逐字保留）。
    const desired = reenableOptionsWithProviderChoice(original.options, {
      author_provider: formOptions.author_provider,
      reviewer_provider: formOptions.reviewer_provider,
    });
    // 序列身份落账：中断后重试据此续跑（enrollment 换了则序列作废）。
    reenableSequenceRef.current = {
      enrollmentId: enrollment.enrollment_id,
      original,
    };

    let current = enrollment;
    // ① 重新开启：source＝当前 card，options/target 回传终止记录（授权选项
    // 不被顺便改写）。旧 source 仍有效但不同（或被并发修改）时服务端 409
    // ——给可行动提示：先去换代/重绑（rebind）更新授权源，再重新开启。
    if (current.terminated_at != null) {
      const command = {
        type: "enable" as const,
        selection_key: reenableSelectionKey(selectionKey),
        source: original.source,
        options: original.options,
        target: original.target,
      };
      try {
        current = await putAutomationEnrollment(projectId, card.issueId, {
          expected_revision: current.policy_revision,
          command: {
            ...command,
            command_id: enableCommandId(JSON.stringify(command)),
          },
        });
      } catch (reason) {
        if (
          reason instanceof ApiRequestError &&
          reason.code === AUTOMATION_CONFLICT_CODE
        ) {
          throw new Error(AUTOMATION_REENABLE_CONFLICT_MESSAGE);
        }
        throw reason;
      }
    }

    // 目标 provider 已生效（未换 provider，或序列已跑到终点）→ 不再改写授权。
    if (current.enabled && sameProviderChoice(current.options, desired)) {
      return;
    }

    // ② 关闭自动化（改授权 payload 的前置步骤；disabled 态重复 Disable 无副作用）。
    if (current.enabled) {
      current = await putAutomationEnrollment(projectId, card.issueId, {
        expected_revision: current.policy_revision,
        command: { type: "disable" },
      });
    }

    // ③ 按新 provider 重新开启（disabled 记录换 payload 重开：契约允许）。
    const command = {
      type: "enable" as const,
      selection_key: reenableSelectionKey(selectionKey),
      source: original.source,
      options: desired,
      target: original.target,
    };
    await putAutomationEnrollment(projectId, card.issueId, {
      expected_revision: current.policy_revision,
      command: {
        ...command,
        command_id: enableCommandId(JSON.stringify(command)),
      },
    });
  }

  async function handleConfirmWorkItemPlanOptions(
    options: WorkItemPlanOptionsFormValue,
  ) {
    if (!selectedProjectId || !pendingWorkItemPlanLaunch) {
      setError("缺少 Project 或 Design Spec");
      return;
    }

    const { card, selectionKey } = pendingWorkItemPlanLaunch;
    if (card.kind !== "design_spec") {
      setError("当前实体不能生成 Work Item Plan");
      return;
    }

    setError(null);
    const { automation_mode: mode, ...planOptions } = options;
    if (mode === "automatic") {
      try {
        await enrollConfirmedDesignAutomation(
          selectedProjectId,
          card,
          selectionKey,
          options,
        );
      } catch (reason) {
        // C5 Task 3：ApiRequestError（code/details）原样上抛——role-chain
        // 预检等结构化错误由弹窗逐角色渲染，不降级为普通 Error 语义。
        if (reason instanceof ApiRequestError) {
          // 批 2 T4（REQ-WIGA-09 修订）：收口未完成有专门的下一步（先重试
          // 终止完成清理），不按通用冲突呈现。
          if (reason.code === AUTOMATION_CLEANUP_REQUIRED_CODE) {
            throw new Error(AUTOMATION_CLEANUP_REQUIRED_MESSAGE);
          }
          throw reason;
        }
        throw new Error(
          errorMessage(reason, AUTOMATION_ENROLL_FAILURE_MESSAGE),
        );
      }
      await refresh(selectedProjectId);
      setPendingWorkItemPlanLaunch(null);
      return;
    }

    const response = await prepareWorkItemPlan(
      selectedProjectId,
      card.issueId,
      {
        title: defaultLaunchTitle({ target: "work_item", card }),
        story_spec_ids: card.raw.story_spec_ids,
        design_spec_ids: [card.id],
        ...planOptions,
      },
    );
    await refresh(selectedProjectId);
    setPendingWorkItemPlanLaunch(null);
    onOpenWorkspace(response.workspace_session.workspace_session_id);
  }

  return {
    handleLaunchWorkspace,
    handleGenerateNext,
    handleGenerateForStage,
    handleConfirmWorkItemPlanOptions,
  };
}
