// Task 9.5：LC 知识库卡片的宿主接线。
// 卡片本体（KnowledgeBaseCard）只负责呈现，本 hook 负责「取数 + 发起动作」：
//   - 抽屉展开且选中 LC 变化时拉 GET .../knowledge-base/status 与
//     GET .../knowledge-base/initializations/active（不在渲染期做副作用，
//     也不在每次渲染重复拉）；
//   - 「知识库初始化」/「知识库更新」→ POST .../knowledge-base/initializations
//     （kind=bootstrap | update；update 必须带 package_path——不带变更包后端
//     无从更新，等于门禁形同虚设）；
//   - 「取消知识库操作」→ POST .../initializations/{operation_id}/cancel；
//   - 「创建输入模板」→ POST .../bootstrap-inputs；
//   - 「查看知识库详细内容」→ 跳 Task 10 的详情页路由。
// 幂等键每次请求新生成（crypto.randomUUID）：后端 store 的 create 无漂移检测，
// 复用同一个键会静默返回旧记录。
import { useCallback, useEffect, useState } from "react";
import { useRouter } from "@tanstack/react-router";
import { ApiRequestError } from "../../api/client";
import {
  cancelKnowledgeBaseOperation,
  createKnowledgeBaseBootstrapInputs,
  getActiveKnowledgeBaseOperation,
  getKnowledgeBaseStatus,
  startKnowledgeBaseInitialization,
} from "../../api/knowledge-base";
import type { RepositoryRegistrationErrorDetails } from "../../api/types";
import type {
  KnowledgeBaseChangePackageViolationDto,
  KnowledgeBaseOperationSnapshotDto,
  KnowledgeBaseStatusDto,
} from "../../api/types/knowledge-base";
import type {
  KnowledgeBasePreflightFailureDetails,
  KnowledgeBasePreflightFailureDto,
} from "./KnowledgeBaseCard";
import { errorMessage } from "./IssueLifecycleWorkbenchParts";

/** 知识库详情页路由（Task 10）。路径逐字固定，参数取当前项目与逻辑代码库。 */
export const KNOWLEDGE_BASE_DETAIL_ROUTE =
  "/workbench/projects/$projectId/logical-codebases/$lcId/knowledge-base";

type Args = {
  selectedProjectId: string | null;
  activeLogicalCodebaseId: string | null;
  /** LC 管理抽屉是否展开：卡片只在展开时挂载，取数时机跟随挂载。 */
  panelExpanded: boolean;
  setError: (message: string | null) => void;
};

/**
 * 后端响应形状守卫：api 客户端不做校验，异常载荷（如代理返回的空对象）不能
 * 直接进卡片——卡片会读 steps/steps.map 等字段，脏数据会让整页崩。
 */
function isKnowledgeBaseStatus(value: unknown): value is KnowledgeBaseStatusDto {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Partial<KnowledgeBaseStatusDto>;
  return typeof candidate.exists === "boolean";
}

function isKnowledgeBaseOperation(
  value: unknown,
): value is KnowledgeBaseOperationSnapshotDto {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Partial<KnowledgeBaseOperationSnapshotDto>;
  return (
    typeof candidate.operation_id === "string" &&
    typeof candidate.status === "string" &&
    Array.isArray(candidate.steps)
  );
}

function isViolation(
  value: unknown,
): value is KnowledgeBaseChangePackageViolationDto {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Partial<KnowledgeBaseChangePackageViolationDto>;
  return typeof candidate.code === "string" && typeof candidate.detail === "string";
}

/**
 * 把 ApiError 的 details 装配成卡片要的预检明细形状（details.missing 是缺失
 * 输入文件相对路径，details.violations 是逐条问题）。口径与卡片自带类型一致。
 */
function toPreflightDetails(
  details: RepositoryRegistrationErrorDetails | undefined,
): KnowledgeBasePreflightFailureDetails | null {
  const missing = Array.isArray(details?.missing)
    ? details.missing.filter((item): item is string => typeof item === "string")
    : undefined;
  const violations = Array.isArray(details?.violations)
    ? details.violations.filter(isViolation)
    : undefined;
  if (missing === undefined && violations === undefined) {
    return null;
  }
  return { missing, violations };
}

/**
 * 预检失败是请求期产物（POST /initializations 被拒时没有 operation 承载），
 * 由宿主把 ApiError 装配成卡片能显示的形状。非 ApiError（网络异常等）不入
 * 预检面，改由工作台级错误横幅呈现。
 */
function toPreflightFailure(
  reason: unknown,
): KnowledgeBasePreflightFailureDto | null {
  if (!(reason instanceof ApiRequestError)) {
    return null;
  }
  return {
    code: reason.code,
    message: reason.message,
    details: toPreflightDetails(reason.details),
  };
}

export function useKnowledgeBaseWorkbench({
  selectedProjectId,
  activeLogicalCodebaseId,
  panelExpanded,
  setError,
}: Args) {
  const [knowledgeBaseStatus, setKnowledgeBaseStatus] =
    useState<KnowledgeBaseStatusDto | null>(null);
  const [knowledgeBaseOperation, setKnowledgeBaseOperation] =
    useState<KnowledgeBaseOperationSnapshotDto | null>(null);
  const [knowledgeBaseBusy, setKnowledgeBaseBusy] = useState(false);
  const [knowledgeBasePreflightFailure, setKnowledgeBasePreflightFailure] =
    useState<KnowledgeBasePreflightFailureDto | null>(null);
  // 无 RouterProvider 时（工作台单测裸渲染）拿不到路由就不导航——与
  // CodingWorkspacePage / workbench-inbox-focus 的既有口径一致。
  const router = useRouter({ warn: false });

  const refresh = useCallback(async () => {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      return;
    }
    try {
      const [status, operation] = await Promise.all([
        getKnowledgeBaseStatus(selectedProjectId, activeLogicalCodebaseId),
        getActiveKnowledgeBaseOperation(selectedProjectId, activeLogicalCodebaseId),
      ]);
      setKnowledgeBaseStatus(isKnowledgeBaseStatus(status) ? status : null);
      setKnowledgeBaseOperation(
        isKnowledgeBaseOperation(operation) ? operation : null,
      );
    } catch (reason) {
      setError(`读取知识库状态失败：${errorMessage(reason, "未知错误")}`);
    }
  }, [selectedProjectId, activeLogicalCodebaseId, setError]);

  // 抽屉展开时取一次；选中 LC 变化（依赖变化）再取一次。收起时不取。
  useEffect(() => {
    if (!panelExpanded) {
      return;
    }
    void refresh();
  }, [panelExpanded, refresh]);

  async function handleStartKnowledgeBase(
    kind: "bootstrap" | "update",
    packagePath?: string,
  ) {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      return;
    }
    setKnowledgeBaseBusy(true);
    try {
      const operation = await startKnowledgeBaseInitialization(
        selectedProjectId,
        activeLogicalCodebaseId,
        kind,
        crypto.randomUUID(),
        packagePath,
      );
      setKnowledgeBaseOperation(operation);
      setKnowledgeBasePreflightFailure(null);
    } catch (reason) {
      const failure = toPreflightFailure(reason);
      if (failure) {
        setKnowledgeBasePreflightFailure(failure);
      }
      setError(
        `知识库${kind === "bootstrap" ? "初始化" : "更新"}失败：${errorMessage(
          reason,
          "未知错误",
        )}`,
      );
    } finally {
      setKnowledgeBaseBusy(false);
    }
  }

  async function handleCancelKnowledgeBase() {
    if (!selectedProjectId || !activeLogicalCodebaseId || !knowledgeBaseOperation) {
      return;
    }
    setKnowledgeBaseBusy(true);
    try {
      const operation = await cancelKnowledgeBaseOperation(
        selectedProjectId,
        activeLogicalCodebaseId,
        knowledgeBaseOperation.operation_id,
      );
      setKnowledgeBaseOperation(operation);
    } catch (reason) {
      setError(`取消知识库操作失败：${errorMessage(reason, "未知错误")}`);
    } finally {
      setKnowledgeBaseBusy(false);
    }
  }

  async function handleCreateKnowledgeBaseBootstrapInputs() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      return;
    }
    setKnowledgeBaseBusy(true);
    try {
      await createKnowledgeBaseBootstrapInputs(
        selectedProjectId,
        activeLogicalCodebaseId,
      );
      setKnowledgeBasePreflightFailure(null);
      await refresh();
    } catch (reason) {
      setError(
        `创建知识库输入模板失败：${errorMessage(reason, "未知错误")}`,
      );
    } finally {
      setKnowledgeBaseBusy(false);
    }
  }

  function handleDismissKnowledgeBasePreflightFailure() {
    setKnowledgeBasePreflightFailure(null);
  }

  function handleOpenKnowledgeBaseDetail() {
    if (!selectedProjectId || !activeLogicalCodebaseId || !router) {
      return;
    }
    void router.navigate({
      to: KNOWLEDGE_BASE_DETAIL_ROUTE,
      params: {
        projectId: selectedProjectId,
        lcId: activeLogicalCodebaseId,
      },
    });
  }

  return {
    knowledgeBaseStatus,
    knowledgeBaseOperation,
    knowledgeBaseBusy,
    knowledgeBasePreflightFailure,
    handleStartKnowledgeBase,
    handleCancelKnowledgeBase,
    handleCreateKnowledgeBaseBootstrapInputs,
    handleDismissKnowledgeBasePreflightFailure,
    handleOpenKnowledgeBaseDetail,
  };
}
