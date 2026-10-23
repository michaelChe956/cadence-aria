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
import type { ProductIssue } from "../../api/types";
import type {
  LifecycleCard,
  LifecycleColumns,
} from "../../state/lifecycle-workbench-store";
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
const AUTOMATION_CLEANUP_REQUIRED_CODE = "automation_plan_cleanup_required";
const AUTOMATION_CLEANUP_REQUIRED_MESSAGE =
  "自动化的终止收口尚未完成：请先重试终止完成清理，再重新开启自动化";

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
  const enableCommandIdRef = useRef<{
    digest: string;
    commandId: string;
  } | null>(null);

  function enableCommandId(payloadDigest: string): string {
    const current = enableCommandIdRef.current;
    if (current && current.digest === payloadDigest) {
      return current.commandId;
    }
    const commandId = newRandomId();
    enableCommandIdRef.current = { digest: payloadDigest, commandId };
    return commandId;
  }

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
   * 批 2 T4（REQ-WIGA-09 修订）：terminated 记录上的人工 Enable 是
   * 「重新开启」——同一条 Enable 命令、同样的线性化 revision，命令身份
   * 换新，payload 原样回传终止记录（不顺便改写授权）。
   */
  async function enrollConfirmedDesignAutomation(
    projectId: string,
    card: Extract<LifecycleCard, { kind: "design_spec" }>,
    selectionKey: string,
    options: WorkItemPlanOptionsFormValue,
  ) {
    const enrollment = await getAutomationEnrollment(projectId, card.issueId);
    // 批 2 T4（REQ-WIGA-09 修订）：terminated 的 enrollment 可被人工显式
    // 重新开启——仍是同一条 Enable 命令，入口（人工点「启用自动化」）即
    // 显式动作，自动路径不触发。并发语义不变：expected_revision 按线性化
    // 带当前 revision；payload 与终止记录逐字段相等（服务端比对，改了即
    // 409）；命令身份（command_id/selection_key）换新——重新开启是新命令，
    // 须能被命令账本单独留痕。
    if (enrollment?.terminated_at != null) {
      const command = {
        type: "enable" as const,
        selection_key: reenableSelectionKey(selectionKey),
        source: enrollment.source,
        options: enrollment.options,
        // 旧代 enrollment 可能没有声明 target（无绑定事实）——此时退回
        // 服务端投影的原样 target，由服务端判定是否接受。
        target:
          enrollment.target ??
          (await getAutomationTarget(projectId, card.issueId, {
            author_provider: options.author_provider,
            reviewer_provider: options.reviewer_provider,
            include_integration_tests: options.include_integration_tests,
            include_e2e_tests: options.include_e2e_tests,
            force_frontend_backend_split: options.force_frontend_backend_split,
            require_execution_plan_confirm:
              options.require_execution_plan_confirm,
          })).enrollment_target,
      };
      await putAutomationEnrollment(projectId, card.issueId, {
        expected_revision: enrollment.policy_revision,
        command: {
          ...command,
          command_id: enableCommandId(JSON.stringify(command)),
        },
      });
      return;
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
