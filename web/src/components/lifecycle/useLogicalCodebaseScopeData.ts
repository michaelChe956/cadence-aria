import { useCallback, useEffect, useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { getActiveAggregateIndex } from "../../api/aggregate-index";
import { getAggregateInitialization } from "../../api/aggregate-initialization";
import { getLogicalCodebaseBootstrap } from "../../api/logical-codebase-bootstrap";
import { listLogicalCodebaseMembers } from "../../api/logicalCodebaseMembers";
import { listPointerPublications } from "../../api/pointer-publication";
import type {
  AggregateIndexActiveResponse,
  AggregateInitializationOperationSnapshot,
  LogicalCodebaseBootstrapProjection,
  LogicalCodebaseMemberDto,
  PointerPublicationDto,
} from "../../api/types";

const POLL_INTERVAL_MS = 2_000;

type UseLogicalCodebaseScopeDataArgs = {
  selectedProjectId: string | null;
  activeLogicalCodebaseId: string | null;
};

type LogicalCodebaseScopeData = {
  logicalCodebaseMembers: LogicalCodebaseMemberDto[];
  setLogicalCodebaseMembers: Dispatch<
    SetStateAction<LogicalCodebaseMemberDto[]>
  >;
  pointerPublications: PointerPublicationDto[];
  setPointerPublications: Dispatch<SetStateAction<PointerPublicationDto[]>>;
  aggregateIndex: AggregateIndexActiveResponse | null;
  setAggregateIndex: Dispatch<
    SetStateAction<AggregateIndexActiveResponse | null>
  >;
  aggregateInitialization: AggregateInitializationOperationSnapshot | null;
  setAggregateInitialization: Dispatch<
    SetStateAction<AggregateInitializationOperationSnapshot | null>
  >;
  bootstrapProjection: LogicalCodebaseBootstrapProjection | null;
  refreshBootstrap: () => void;
  latestPointerPublication: PointerPublicationDto | null;
  showIncrementalHint: boolean;
};

/**
 * 选中 LC 作用域的数据加载 hook（纯搬运自 IssueLifecycleWorkbench，无行为改动）：
 * 成员/指针发布/聚合索引按选中 LC 拉取 + 指针发布/聚合初始化轮询。
 *
 * - R8：多 LC 并存时面板数据随选中态切换。
 * - I1：LC/Project 切换时同步重置 aggregateInitialization，避免残留上一 LC 的
 *   operation（其轮询会带着陈旧 operation_id 打到新 LC 路径上）。
 */
export function useLogicalCodebaseScopeData({
  selectedProjectId,
  activeLogicalCodebaseId,
}: UseLogicalCodebaseScopeDataArgs): LogicalCodebaseScopeData {
  const [pointerPublications, setPointerPublications] = useState<
    PointerPublicationDto[]
  >([]);
  const [logicalCodebaseMembers, setLogicalCodebaseMembers] = useState<
    LogicalCodebaseMemberDto[]
  >([]);
  const [aggregateIndex, setAggregateIndex] =
    useState<AggregateIndexActiveResponse | null>(null);
  const [aggregateInitialization, setAggregateInitialization] =
    useState<AggregateInitializationOperationSnapshot | null>(null);
  const [bootstrapProjection, setBootstrapProjection] =
    useState<LogicalCodebaseBootstrapProjection | null>(null);
  const [bootstrapRefreshTick, setBootstrapRefreshTick] = useState(0);

  const latestPointerPublication = useMemo<PointerPublicationDto | null>(() => {
    if (pointerPublications.length === 0) {
      return null;
    }
    return [...pointerPublications].sort((left, right) =>
      right.created_at.localeCompare(left.created_at),
    )[0];
  }, [pointerPublications]);

  const latestPointerPublicationId =
    latestPointerPublication?.status === "in_progress"
      ? latestPointerPublication.id
      : null;
  const latestCompletedPointerPublication =
    useMemo<PointerPublicationDto | null>(() => {
      const completed = pointerPublications.filter(
        (publication) => publication.status !== "in_progress",
      );
      if (completed.length === 0) {
        return null;
      }
      return [...completed].sort((left, right) =>
        right.created_at.localeCompare(left.created_at),
      )[0];
    }, [pointerPublications]);
  const showIncrementalHint =
    latestCompletedPointerPublication !== null &&
    logicalCodebaseMembers.length >
      latestCompletedPointerPublication.entries.length;

  useEffect(() => {
    setAggregateInitialization(null);
    setBootstrapProjection(null);
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setLogicalCodebaseMembers([]);
      setPointerPublications([]);
      setAggregateIndex(null);
      return;
    }
    let disposed = false;
    (async () => {
      try {
        const [membersResponse, publicationResponse] = await Promise.all([
          listLogicalCodebaseMembers(
            selectedProjectId,
            activeLogicalCodebaseId,
          ),
          listPointerPublications(selectedProjectId, activeLogicalCodebaseId),
        ]);
        if (disposed) {
          return;
        }
        setLogicalCodebaseMembers(membersResponse.members ?? []);
        setPointerPublications(publicationResponse ?? []);
        setAggregateIndex(
          (membersResponse.members ?? []).length > 0
            ? await getActiveAggregateIndex(
                selectedProjectId,
                activeLogicalCodebaseId,
              )
            : null,
        );
      } catch {
        // LC 作用域数据加载失败保持现状，交由全局 refresh 重试。
      }
    })();
    return () => {
      disposed = true;
    };
  }, [selectedProjectId, activeLogicalCodebaseId]);

  // C4 Task 9：选中 LC 的 bootstrap 纯投影拉取/刷新（动作成功后经
  // refreshBootstrap 立即补读；GET 本身零副作用）。SSE/失效唤醒由全局
  // 目录刷新汇入，这里只维护当前 LC 的等待面。
  useEffect(() => {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      return;
    }
    let disposed = false;
    (async () => {
      try {
        const projection = await getLogicalCodebaseBootstrap(
          selectedProjectId,
          activeLogicalCodebaseId,
        );
        if (!disposed && projection !== null && Array.isArray(projection.steps)) {
          setBootstrapProjection(projection);
        }
      } catch {
        // bootstrap 投影读取失败保持现状，交由轮询/全局 refresh 重试。
      }
    })();
    return () => {
      disposed = true;
    };
  }, [selectedProjectId, activeLogicalCodebaseId, bootstrapRefreshTick]);

  // 冷启动进行中（任一步骤 Running）时按固定间隔轮询；到达稳定态
  // （planning_ready 或全部 Failed/等待人工）即停止，等待显式动作唤醒。
  const bootstrapPolling =
    bootstrapProjection !== null &&
    !bootstrapProjection.planning_ready &&
    (bootstrapProjection.steps ?? []).some((step) => step.status === "running");
  useEffect(() => {
    if (
      !selectedProjectId ||
      !activeLogicalCodebaseId ||
      !bootstrapPolling
    ) {
      return;
    }
    const interval = window.setInterval(() => {
      setBootstrapRefreshTick((tick) => tick + 1);
    }, POLL_INTERVAL_MS);
    return () => {
      window.clearInterval(interval);
    };
  }, [selectedProjectId, activeLogicalCodebaseId, bootstrapPolling]);

  const refreshBootstrap = useCallback(() => {
    setBootstrapRefreshTick((tick) => tick + 1);
  }, []);

  useEffect(() => {
    if (
      !selectedProjectId ||
      !activeLogicalCodebaseId ||
      !latestPointerPublicationId
    ) {
      return;
    }

    let disposed = false;
    let inFlight = false;
    const poll = async () => {
      if (inFlight) {
        return;
      }
      inFlight = true;
      try {
        const publications = await listPointerPublications(
          selectedProjectId,
          activeLogicalCodebaseId,
        );
        if (!disposed) {
          setPointerPublications(publications);
        }
      } catch {
        // 轮询失败保持现状，下一次间隔重试。
      } finally {
        inFlight = false;
      }
    };
    const interval = window.setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [latestPointerPublicationId, selectedProjectId, activeLogicalCodebaseId]);

  useEffect(() => {
    if (
      !selectedProjectId ||
      !activeLogicalCodebaseId ||
      !aggregateInitialization ||
      (aggregateInitialization.status !== "created" &&
        aggregateInitialization.status !== "running")
    ) {
      return;
    }

    let disposed = false;
    let inFlight = false;
    const poll = async () => {
      if (inFlight) {
        return;
      }
      inFlight = true;
      try {
        const operation = await getAggregateInitialization(
          selectedProjectId,
          activeLogicalCodebaseId,
          aggregateInitialization.operation_id,
        );
        if (!disposed) {
          setAggregateInitialization(operation);
        }
      } catch {
        // 轮询失败保持现状，下一次间隔重试。
      } finally {
        inFlight = false;
      }
    };
    const interval = window.setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [
    selectedProjectId,
    activeLogicalCodebaseId,
    aggregateInitialization?.operation_id,
    aggregateInitialization?.status,
  ]);

  return {
    logicalCodebaseMembers,
    setLogicalCodebaseMembers,
    pointerPublications,
    setPointerPublications,
    aggregateIndex,
    setAggregateIndex,
    aggregateInitialization,
    setAggregateInitialization,
    bootstrapProjection,
    refreshBootstrap,
    latestPointerPublication,
    showIncrementalHint,
  };
}
