// 从 IssueLifecycleWorkbench.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 生命周期实体删除动作（Issue / Story / Design / Work Item / 计划 Coding 工作区）。
import type { Dispatch, RefObject, SetStateAction } from "react";
import {
  ApiRequestError,
  deleteDesignSpec,
  deleteProductIssue,
  deleteStorySpec,
  deleteWorkItem,
  deleteWorkItemPlan,
  deletePlanCodingWorkspaces,
  getPlanCodingProgress,
  type DeleteLifecycleEntityOptions,
} from "../../api/client";
import type { ProductIssue } from "../../api/types";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import {
  errorMessage,
  lifecycleCardKey,
  lifecycleEntityKey,
  waitForDeleteExitAnimation,
} from "./IssueLifecycleWorkbenchParts";

type Args = {
  selectedProjectId: string | null;
  focusedIssueId: string | null;
  selectedCardKey: string | null;
  drawerFocusedEntityKey: string | null;
  groupCodingDeleteBusy: boolean;
  planGroupDeleteRef: RefObject<{ planId: string; commandId: string } | null>;
  setError: Dispatch<SetStateAction<string | null>>;
  setFocusedIssueId: Dispatch<SetStateAction<string | null>>;
  setSelectedCardKey: Dispatch<SetStateAction<string | null>>;
  setDeletingCardKey: Dispatch<SetStateAction<string | null>>;
  setGroupCodingDeleteBusy: Dispatch<SetStateAction<boolean>>;
  closeDrawer: () => void;
  refresh: (
    projectIdOverride?: string | null,
    optimisticIssues?: readonly ProductIssue[],
  ) => Promise<void>;
};

export function useIssueLifecycleWorkbenchDeletion({
  selectedProjectId,
  focusedIssueId,
  selectedCardKey,
  drawerFocusedEntityKey,
  groupCodingDeleteBusy,
  planGroupDeleteRef,
  setError,
  setFocusedIssueId,
  setSelectedCardKey,
  setDeletingCardKey,
  setGroupCodingDeleteBusy,
  closeDrawer,
  refresh,
}: Args) {
    async function handleDeleteIssue(issueId: string) {
      if (!selectedProjectId) {
        setError("缺少 Project");
        return;
      }

      const cardKey = lifecycleEntityKey("issue", issueId, issueId);
      setDeletingCardKey(cardKey);
      setError(null);
      try {
        await Promise.all([
          deleteProductIssue(selectedProjectId, issueId),
          waitForDeleteExitAnimation(),
        ]);
        if (focusedIssueId === issueId) {
          setFocusedIssueId(null);
        }
        setSelectedCardKey(null);
        await refresh(selectedProjectId);
      } catch (reason) {
        setError(errorMessage(reason, "删除 Issue 失败"));
      } finally {
        setDeletingCardKey(null);
      }
    }

    async function handleDeleteLifecycleCard(card: LifecycleCardData) {
      if (!selectedProjectId) {
        setError("缺少 Project");
        return;
      }
      if (
        card.kind !== "story_spec" &&
        card.kind !== "design_spec" &&
        card.kind !== "work_item" &&
        card.kind !== "work_item_group"
      ) {
        setError("Issue 请从 Issue 卡片列表删除");
        return;
      }

      // 级联删除 D2.3：删除请求按需显式发起（可二次调用），不是预创建 Promise；
      // 确认资格由服务端三分类（D1.4）权威判定，前端不做确认历史判断。
      const kind = card.kind;
      const sendDelete = (
        options?: DeleteLifecycleEntityOptions,
      ): Promise<{ status: string }> => {
        if (kind === "story_spec") {
          return deleteStorySpec(selectedProjectId, card.issueId, card.id, options);
        }
        if (kind === "design_spec") {
          return deleteDesignSpec(selectedProjectId, card.issueId, card.id, options);
        }
        if (kind === "work_item") {
          // 独立 WI 非本次级联确认语义：保原行为（无 confirmed query）。
          return deleteWorkItem(selectedProjectId, card.issueId, card.id);
        }
        return deleteWorkItemPlan(selectedProjectId, card.issueId, card.id, options);
      };

      const cardKey = lifecycleCardKey(card);
      setDeletingCardKey(cardKey);
      setError(null);
      try {
        try {
          await Promise.all([sendDelete(), waitForDeleteExitAnimation()]);
        } catch (reason) {
          // required 是唯一弹窗分支：409 deletion_confirmation_required 恰一次
          // 确认，同意后第二 delete 显式带 confirmed=true；取消零出站破坏请求、
          // 无刷新/退场。其余错误（coding/reference/busy 等）沿原错误模式提示
          // 先删后置，不自动重试、不弹窗。
          if (
            !(
              reason instanceof ApiRequestError &&
              reason.code === "deletion_confirmation_required"
            )
          ) {
            throw reason;
          }
          const confirmed = window.confirm(
            "确定永久删除？此操作将永久清除内容、工作项、会话和 review，且不可恢复。",
          );
          if (!confirmed) {
            return;
          }
          await Promise.all([
            sendDelete({ confirmed: true }),
            waitForDeleteExitAnimation(),
          ]);
        }
        if (selectedCardKey === cardKey) {
          setSelectedCardKey(null);
        }
        if (drawerFocusedEntityKey === cardKey) {
          closeDrawer();
        }
        await refresh(selectedProjectId);
      } catch (reason) {
        setError(errorMessage(reason, "删除生命周期实体失败"));
      } finally {
        setDeletingCardKey(null);
      }
    }

    function handleDeleteLifecycleCardFromDrawer(card: LifecycleCardData) {
      if (card.kind !== "work_item" && card.kind !== "work_item_group") {
        return;
      }
      if (card.kind === "work_item_group") {
        // group drawer 移除旧无条件确认：Story/Design/group 同走服务端单次确认流。
        void handleDeleteLifecycleCard(card);
        return;
      }
      // 独立 WI 保原确认行为，但文案不再声称自动删除 Coding Workspace（存在时
      // 服务端以 coding_workspace_exists 拒删，需先删除后置对象）。
      const confirmed = window.confirm(
        "删除 Work Item 会同时删除其会话与日志；若存在关联 Coding Workspace 需先删除，且无法撤销。",
      );
      if (!confirmed) {
        return;
      }
      void handleDeleteLifecycleCard(card);
    }

    // H5（设计 C，冻结契约先行、H4 端点后行）：plan 抽屉 Coding 区辅入口——与
    // CodingWorkspacePage 页级入口同一 DELETE …/coding-workspaces 契约：服务端
    // 分类 → required 409 → 恰一次 confirm → confirmed=true 复删（同 command_id、
    // GET workspace_lifecycle.generation 冻结重用）；取消零出站破坏请求；typed
    // 409 直达错误面不弹窗不重试；成功刷新 lifecycle（计划与工作项保留，抽屉不关）。
    async function handleDeleteAllCodingWorkspacesFromDrawer(card: LifecycleCardData) {
      if (!selectedProjectId || card.kind !== "work_item_group" || groupCodingDeleteBusy) {
        return;
      }
      const planId = card.id;
      const previous = planGroupDeleteRef.current;
      const commandId =
        previous?.planId === planId
          ? previous.commandId
          : `plan_delete_ui_${
              typeof crypto !== "undefined" && "randomUUID" in crypto
                ? crypto.randomUUID()
                : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
            }`;
      planGroupDeleteRef.current = { planId, commandId };
      let generation: number | null = null;
      let generationResolved = false;
      const resolveGeneration = async () => {
        if (generationResolved) {
          return;
        }
        generationResolved = true;
        try {
          const progress = await getPlanCodingProgress(
            selectedProjectId,
            card.issueId,
            planId,
          );
          generation = progress.workspace_lifecycle?.generation ?? null;
        } catch {
          // 聚合缺席不阻断服务端预检：省略 generation，服务端分类权威。
          generation = null;
        }
      };
      const sendDelete = async (confirmed: boolean) => {
        await resolveGeneration();
        return deletePlanCodingWorkspaces(selectedProjectId, card.issueId, planId, {
          command_id: commandId,
          ...(generation !== null ? { expected_generation: generation } : {}),
          ...(confirmed ? { confirmed: true } : {}),
        });
      };
      setGroupCodingDeleteBusy(true);
      setError(null);
      try {
        try {
          await sendDelete(false);
        } catch (reason) {
          if (
            !(
              reason instanceof ApiRequestError &&
              reason.code === "deletion_confirmation_required"
            )
          ) {
            throw reason;
          }
          const confirmationMessage =
            reason.message.trim().length > 0
              ? reason.message
              : "确定终止并删除本计划全部 Coding 工作区？运行中的成员会先终止，日志、测试输出与 worktree 将被删除且不可恢复。计划与工作项保留。";
          if (!window.confirm(confirmationMessage)) {
            return;
          }
          await sendDelete(true);
        }
        await refresh(selectedProjectId);
      } catch (reason) {
        setError(errorMessage(reason, "删除本计划全部 Coding 工作区失败"));
      } finally {
        setGroupCodingDeleteBusy(false);
      }
    }

  return {
    handleDeleteIssue,
    handleDeleteLifecycleCard,
    handleDeleteLifecycleCardFromDrawer,
    handleDeleteAllCodingWorkspacesFromDrawer,
  };
}
