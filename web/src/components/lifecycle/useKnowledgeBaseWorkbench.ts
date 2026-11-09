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
// 取数与动作都带「过期守卫」：LC 切换或更新的动作结果已落盘时，在飞请求的
// 旧响应一律丢弃（否则旧 LC 的数据会挂到当前选中的 LC 上，展开时的旧取数也会
// 用「无 active operation」抹掉刚启动的 running operation）。
import { useCallback, useEffect, useRef, useState } from "react";
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

  // 作用域（项目 + LC）：渲染期同步到 ref，供在飞请求回来时判断自己是否过期。
  // （渲染期写 ref ⇒ 提交与副作用之间没有时间窗，晚到的旧响应也不会被误收。）
  const scopeKey = `${selectedProjectId ?? ""}|${activeLogicalCodebaseId ?? ""}`;
  const scopeRef = useRef(scopeKey);
  scopeRef.current = scopeKey;
  // 上一作用域：识别「LC/项目切换了」，切换时清空上一 LC 的数据。
  const previousScopeRef = useRef(scopeKey);
  // 状态版本号：每次「准备写入知识库状态」（发起取数 / 动作结果落盘）都推进；
  // 在飞请求回来时版本已变 ⇒ 它的响应已过期（LC 已切换，或已被更新的动作结果
  // 取代），直接丢弃。没有这个守卫时：切 LC A→B 会用 A 的慢响应覆盖 B 的卡片
  // （把 A 的状态/operation 归属到当前选中的 B 上）；启动后展开时触发的旧取数
  // 也会用「无 active operation」抹掉刚设置的 running operation（用户会误以为
  // 启动失败，而后端其实在跑）。
  const revisionRef = useRef(0);

  const refresh = useCallback(async () => {
    const projectId = selectedProjectId;
    const lcId = activeLogicalCodebaseId;
    if (!projectId || !lcId) {
      return;
    }
    const scope = scopeKey;
    revisionRef.current += 1;
    const revision = revisionRef.current;
    try {
      const [status, operation] = await Promise.all([
        getKnowledgeBaseStatus(projectId, lcId),
        getActiveKnowledgeBaseOperation(projectId, lcId),
      ]);
      // 过期守卫：期间 LC 已切换（scope 变了）或状态已被更新的动作结果取代
      // （版本号变了）⇒ 丢弃本次响应，不让它覆盖当前状态。
      if (revision !== revisionRef.current || scope !== scopeRef.current) {
        return;
      }
      setKnowledgeBaseStatus(isKnowledgeBaseStatus(status) ? status : null);
      setKnowledgeBaseOperation(
        isKnowledgeBaseOperation(operation) ? operation : null,
      );
    } catch (reason) {
      if (revision !== revisionRef.current || scope !== scopeRef.current) {
        return;
      }
      setError(`读取知识库状态失败：${errorMessage(reason, "未知错误")}`);
    }
  }, [selectedProjectId, activeLogicalCodebaseId, scopeKey, setError]);

  // LC/项目切换时清空上一 LC 的数据：避免切换瞬间闪现前一 LC 的状态，也避免
  // A 的预检失败挂在 B 的卡片上。取数由下面的取数 effect 重新拉。
  useEffect(() => {
    if (previousScopeRef.current === scopeKey) {
      return;
    }
    previousScopeRef.current = scopeKey;
    setKnowledgeBaseStatus(null);
    setKnowledgeBaseOperation(null);
    setKnowledgeBasePreflightFailure(null);
  }, [scopeKey]);

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
    const scope = scopeKey;
    setKnowledgeBaseBusy(true);
    try {
      const operation = await startKnowledgeBaseInitialization(
        selectedProjectId,
        activeLogicalCodebaseId,
        kind,
        crypto.randomUUID(),
        packagePath,
      );
      if (scope !== scopeRef.current) {
        // LC 已切换：过期响应不落到当前卡片上。
        return;
      }
      // 推进版本号：作废在飞的旧取数——它若晚到会用「无 active operation」覆盖
      // 这里刚设置的 running operation（用户会误以为启动失败，而后端在跑）。
      revisionRef.current += 1;
      // 形状守卫与取数侧一致：200 + 异常载荷（如代理返回空对象）不能直接进卡片。
      setKnowledgeBaseOperation(
        isKnowledgeBaseOperation(operation) ? operation : null,
      );
      setKnowledgeBasePreflightFailure(null);
    } catch (reason) {
      if (scope !== scopeRef.current) {
        return;
      }
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
    const scope = scopeKey;
    setKnowledgeBaseBusy(true);
    try {
      const operation = await cancelKnowledgeBaseOperation(
        selectedProjectId,
        activeLogicalCodebaseId,
        knowledgeBaseOperation.operation_id,
      );
      if (scope !== scopeRef.current) {
        return;
      }
      revisionRef.current += 1;
      setKnowledgeBaseOperation(
        isKnowledgeBaseOperation(operation) ? operation : null,
      );
    } catch (reason) {
      if (scope !== scopeRef.current) {
        return;
      }
      setError(`取消知识库操作失败：${errorMessage(reason, "未知错误")}`);
    } finally {
      setKnowledgeBaseBusy(false);
    }
  }

  async function handleCreateKnowledgeBaseBootstrapInputs() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      return;
    }
    const scope = scopeKey;
    setKnowledgeBaseBusy(true);
    try {
      await createKnowledgeBaseBootstrapInputs(
        selectedProjectId,
        activeLogicalCodebaseId,
      );
      if (scope !== scopeRef.current) {
        return;
      }
      setKnowledgeBasePreflightFailure(null);
      await refresh();
    } catch (reason) {
      if (scope !== scopeRef.current) {
        return;
      }
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
