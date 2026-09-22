import type { Dispatch, SetStateAction } from "react";
import {
  generateDesignSpecs,
  generateStorySpecs,
  prepareWorkItemPlan,
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
import type { WorkItemPlanOptionsFormValue } from "./WorkItemPlanOptionsDialog";

export type ProviderWorkspaceLaunchTarget = "story" | "design" | "work_item";

export type PendingWorkItemPlanLaunch = {
  card: LifecycleCard;
  /** REQ-PPS-01：开启弹窗即快照表单初值（含用户默认 provider），弹窗期间不被并发写入改动。 */
  options: WorkItemPlanOptionsFormValue;
};

const DEFAULT_WORK_ITEM_PLAN_OPTIONS = {
  include_integration_tests: true,
  include_e2e_tests: false,
  force_frontend_backend_split: true,
  require_execution_plan_confirm: false,
} satisfies WorkItemPlanOptionsFormValue;

// REQ-PPS-01 场景三：创建被服务端 fail-closed 拒绝（provider 不可用等 4xx）时的兜底文案，
// 服务端 message 存在时以它为准。
const LAUNCH_FAILURE_MESSAGES: Record<ProviderWorkspaceLaunchTarget, string> = {
  story: "生成 Story Spec 失败",
  design: "生成 Design Spec 失败",
  work_item: "生成 Work Item Plan 失败",
};

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
        setPendingWorkItemPlanLaunch({
          card,
          options: {
            ...DEFAULT_WORK_ITEM_PLAN_OPTIONS,
            ...readWorkspaceProviderDefaultsSnapshot(),
          },
        });
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
        setPendingWorkItemPlanLaunch({
          card,
          options: {
            ...DEFAULT_WORK_ITEM_PLAN_OPTIONS,
            ...readWorkspaceProviderDefaultsSnapshot(),
          },
        });
        return;
      }

      setError("当前卡片不能启动该 Workspace");
    } catch (reason) {
      // 调用方是 `void handleLaunchWorkspace(...)`：没有宿主接住这条链路，失败必须
      // 落到工作台错误横幅，否则 provider 不可用等 4xx 对用户完全不可见。
      setError(errorMessage(reason, LAUNCH_FAILURE_MESSAGES[target]));
    }
  }

  async function handleConfirmWorkItemPlanOptions(
    options: WorkItemPlanOptionsFormValue,
  ) {
    if (!selectedProjectId || !pendingWorkItemPlanLaunch) {
      setError("缺少 Project 或 Design Spec");
      return;
    }

    const { card } = pendingWorkItemPlanLaunch;
    if (card.kind !== "design_spec") {
      setError("当前实体不能生成 Work Item Plan");
      return;
    }

    setError(null);
    const response = await prepareWorkItemPlan(
      selectedProjectId,
      card.issueId,
      {
        title: defaultLaunchTitle({ target: "work_item", card }),
        story_spec_ids: card.raw.story_spec_ids,
        design_spec_ids: [card.id],
        ...options,
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
