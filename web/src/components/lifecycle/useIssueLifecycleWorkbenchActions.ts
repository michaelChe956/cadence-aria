import { useEffect, useRef, type Dispatch, type SetStateAction } from "react";
import { ApiRequestError, deleteProject, deleteRepository } from "../../api/client";
import { getActiveAggregateIndex, rebuildAggregateIndex } from "../../api/aggregate-index";
import { cancelAggregateInitialization, reinitializeAggregateInitialization, startAggregateInitialization } from "../../api/aggregate-initialization";
import { deleteLogicalCodebase, purgeLogicalCodebase, restoreLogicalCodebase } from "../../api/codebases";
import { createPointerPublication, retryPointerPublicationRepo, revokePointerPublication } from "../../api/pointer-publication";
import type {
  AggregateIndexActiveResponse,
  AggregateInitializationOperationSnapshot,
  PointerPublicationDto,
  RepositoryInitializationOperationSnapshot,
} from "../../api/types";
import { errorMessage } from "./IssueLifecycleWorkbenchParts";

type Args = {
  selectedProjectId: string | null;
  activeLogicalCodebaseId: string | null;
  latestPointerPublication: PointerPublicationDto | null;
  aggregateInitialization: AggregateInitializationOperationSnapshot | null;
  selectedLogicalCodebaseId: string | null;
  setSelectedCardKey: Dispatch<SetStateAction<string | null>>;
  setSelectedLogicalCodebaseId: Dispatch<SetStateAction<string | null>>;
  setError: Dispatch<SetStateAction<string | null>>;
  setPointerPublicationBusy: Dispatch<SetStateAction<boolean>>;
  setPointerPublications: Dispatch<SetStateAction<PointerPublicationDto[]>>;
  setAggregateIndex: Dispatch<SetStateAction<AggregateIndexActiveResponse | null>>;
  setAggregateIndexRebuilding: Dispatch<SetStateAction<boolean>>;
  setAggregateInitialization: Dispatch<SetStateAction<AggregateInitializationOperationSnapshot | null>>;
  setAggregateInitializationBusy: Dispatch<SetStateAction<boolean>>;
  refresh: (projectId?: string | null) => Promise<void>;
};

export function useIssueLifecycleWorkbenchActions({
  selectedProjectId, activeLogicalCodebaseId, latestPointerPublication,
  aggregateInitialization, selectedLogicalCodebaseId, setSelectedCardKey,
  setSelectedLogicalCodebaseId, setError,
  setPointerPublicationBusy, setPointerPublications, setAggregateIndex, setAggregateIndexRebuilding,
  setAggregateInitialization, setAggregateInitializationBusy, refresh,
}: Args) {
  // 2026-11-19：重建聚合索引的在途失效令牌与挂载位——作用域切换（含 A→B→A）
  // 或新一次重建开始都会作废旧请求的全部逻辑写入（busy/错误/卡片）；卸载后
  // 同样不再回写（fetch 层无中止通道，只拦逻辑写入）。
  const rebuildRequestIdRef = useRef(0);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);
  useEffect(() => {
    // 作用域切换：作废在途重建并复位重建 busy，避免旧 LC 的进行态黏在
    // 新选中 LC 的「重建索引」按钮上。
    rebuildRequestIdRef.current += 1;
    setAggregateIndexRebuilding(false);
  }, [selectedProjectId, activeLogicalCodebaseId, setAggregateIndexRebuilding]);

  function upsertPointerPublication(publication: PointerPublicationDto) {
    setPointerPublications((existing) => [
      ...existing.filter((item) => item.id !== publication.id),
      publication,
    ]);
  }

  async function handlePublish(kind: "full" | "incremental") {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setError("缺少 Project 或逻辑代码库");
      return;
    }
    setPointerPublicationBusy(true);
    setError(null);
    try {
      const publication = await createPointerPublication(selectedProjectId, activeLogicalCodebaseId, kind);
      upsertPointerPublication(publication);
      return publication;
    } catch (reason) {
      setError(errorMessage(reason, kind === "full" ? "全量发布失败" : "增量发布失败"));
      return null;
    } finally {
      setPointerPublicationBusy(false);
    }
  }

  async function handleRetryRepo(memberRepoId: string) {
    if (!selectedProjectId || !activeLogicalCodebaseId || !latestPointerPublication) {
      setError("缺少 Project 或发布批次");
      return null;
    }
    setPointerPublicationBusy(true);
    setError(null);
    try {
      const publication = await retryPointerPublicationRepo(selectedProjectId, activeLogicalCodebaseId, latestPointerPublication.id, memberRepoId);
      upsertPointerPublication(publication);
      return publication;
    } catch (reason) {
      setError(errorMessage(reason, "重试失败"));
      return null;
    } finally {
      setPointerPublicationBusy(false);
    }
  }

  async function handleRevokePublication() {
    if (!selectedProjectId || !activeLogicalCodebaseId || !latestPointerPublication) {
      setError("缺少 Project 或发布批次");
      return null;
    }
    setPointerPublicationBusy(true);
    setError(null);
    try {
      const publication = await revokePointerPublication(selectedProjectId, activeLogicalCodebaseId, latestPointerPublication.id);
      upsertPointerPublication(publication);
      return publication;
    } catch (reason) {
      setError(errorMessage(reason, "撤回失败"));
      return null;
    } finally {
      setPointerPublicationBusy(false);
    }
  }

  // 2026-11-19：重建失败后的卡片回读——客户实测截图显示 POST 失败后卡片仍残留
  // 旧告警（此前 catch 只 setError）。失败时对本次捕获的作用域补读一次 GET
  // active 刷新卡片；补读失败保留原 POST 错误与旧卡片，不重试、不轮询。
  // 成功路径直接采用 POST 响应，不额外 GET。
  async function handleRebuildAggregateIndex() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setError("缺少 Project 或逻辑代码库");
      return;
    }
    const projectId = selectedProjectId;
    const logicalCodebaseId = activeLogicalCodebaseId;
    const requestId = rebuildRequestIdRef.current + 1;
    rebuildRequestIdRef.current = requestId;
    // 写入仅在：组件仍挂载，且本次请求仍是最新一次重建（作用域切换或新一次
    // 重建都会使旧请求的 busy/错误/卡片写入全部作废）。
    const canWrite = () =>
      mountedRef.current && requestId === rebuildRequestIdRef.current;
    setAggregateIndexRebuilding(true);
    setError(null);
    try {
      const rebuilt = await rebuildAggregateIndex(projectId, logicalCodebaseId);
      if (!canWrite()) {
        return;
      }
      setAggregateIndex(rebuilt);
    } catch (reason) {
      if (!canWrite()) {
        return;
      }
      setError(reason instanceof ApiRequestError ? reason.message : errorMessage(reason, "重建聚合索引失败"));
      try {
        const active = await getActiveAggregateIndex(projectId, logicalCodebaseId);
        if (canWrite()) {
          setAggregateIndex(active);
        }
      } catch {
        // 补读失败：保留原 POST 错误与旧卡片。
      }
    } finally {
      if (canWrite()) {
        setAggregateIndexRebuilding(false);
      }
    }
  }

  async function handleStartAggregateInitialization() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setError("缺少 Project 或逻辑代码库");
      return;
    }
    setAggregateInitializationBusy(true);
    setError(null);
    try {
      setAggregateInitialization(await startAggregateInitialization(selectedProjectId, activeLogicalCodebaseId, crypto.randomUUID()));
    } catch (reason) {
      setError(reason instanceof ApiRequestError ? reason.message : errorMessage(reason, "启动聚合初始化失败"));
    } finally {
      setAggregateInitializationBusy(false);
    }
  }

  async function handleCancelAggregateInitialization() {
    if (!selectedProjectId || !activeLogicalCodebaseId || !aggregateInitialization) {
      setError("缺少 Project 或初始化操作");
      return;
    }
    setAggregateInitializationBusy(true);
    setError(null);
    try {
      setAggregateInitialization(await cancelAggregateInitialization(selectedProjectId, activeLogicalCodebaseId, aggregateInitialization.operation_id, { reason: "user_cancelled", detail: null }));
    } catch (reason) {
      setError(reason instanceof ApiRequestError ? reason.message : errorMessage(reason, "取消聚合初始化失败"));
    } finally {
      setAggregateInitializationBusy(false);
    }
  }

  /**
   * 「重新初始化」：走 POST /reinitializations——服务端在创建期冻结「强制跑
   * 完整五步」事实（无视聚合根是否已做过 Cadence-skills 初始化），适用于
   * Cadence-skills 更新后重装。幂等键每次新生成（与初始化入口同构）。
   */
  async function handleReinitializeAggregateInitialization() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setError("缺少 Project 或逻辑代码库");
      return;
    }
    setAggregateInitializationBusy(true);
    setError(null);
    try {
      setAggregateInitialization(await reinitializeAggregateInitialization(selectedProjectId, activeLogicalCodebaseId, crypto.randomUUID()));
    } catch (reason) {
      setError(reason instanceof ApiRequestError ? reason.message : errorMessage(reason, "重新初始化失败"));
    } finally {
      setAggregateInitializationBusy(false);
    }
  }

  async function handleDeleteProject(projectId: string) {
    // 2026-11-05 真机事故回归：侧栏垃圾桶一次点击曾直接硬删整个项目工作区
    //（项目 + 全部逻辑代码库 + 成员登记 + capability 记录）。对齐逻辑代码库
    // 删除/清除的二次确认形态：先弹强警告，取消零副作用；确认后 delete 才
    // 带 confirmed=true，后端 409 deletion_confirmation_required 强制兜底。
    const confirmed = window.confirm(
      "确定删除该 Project？将删除整个项目及其全部逻辑代码库、成员登记与 capability 记录，且不可恢复。",
    );
    if (!confirmed) return;
    setError(null);
    try {
      await deleteProject(projectId, { confirmed: true });
      await refresh();
    } catch (reason) {
      setError(errorMessage(reason, "删除 Project 失败"));
    }
  }

  async function handleDeleteRepository(repositoryId: string) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }
    setError(null);
    await deleteRepository(selectedProjectId, repositoryId, `delete-repository-${crypto.randomUUID()}`);
    setSelectedCardKey(null);
    await refresh(selectedProjectId);
  }

  async function handleDeleteLogicalCodebase(logicalCodebaseId: string) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }
    // Bug3 ③：软删除语义如实告知——数据与成员仓保留、可经恢复端点还原
    //（POST /logical-codebases/{id}/restore），不再误称「无法撤销」。
    const confirmed = window.confirm(
      "删除逻辑代码库为软删除：仅从列表隐藏并释放成员登记，成员仓、聚合索引与指针发布上下文保留原样且成员仓 Git 零改动，可经恢复端点还原。确认删除？",
    );
    if (!confirmed) return;
    setError(null);
    try {
      await deleteLogicalCodebase(selectedProjectId, logicalCodebaseId);
      if (selectedLogicalCodebaseId === logicalCodebaseId) setSelectedLogicalCodebaseId(null);
      await refresh(selectedProjectId);
    } catch (reason) {
      setError(errorMessage(reason, "删除逻辑代码库失败"));
    }
  }

  // 2026-10-29：已软删 LC 的恢复——摘墓碑复原原子树（id 不变，引用不断裂）。
  async function handleRestoreLogicalCodebase(logicalCodebaseId: string) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }
    setError(null);
    try {
      await restoreLogicalCodebase(selectedProjectId, logicalCodebaseId);
      await refresh(selectedProjectId);
    } catch (reason) {
      setError(errorMessage(reason, "恢复逻辑代码库失败"));
    }
  }

  // 2026-10-29：彻底删除（硬删）——不可恢复。二次确认必须写明清理范围与
  // 边界（清政策产物与成员指针，不动 Cadence-skills 文件）。
  async function handlePurgeLogicalCodebase(logicalCodebaseId: string) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }
    const confirmed = window.confirm(
      "彻底删除不可恢复：将永久删除该逻辑代码库的应用数据、工作区里的政策产物（policy/<project>/<logical_codebase>）与成员仓的 .aria-pointer.md 指针文件；不会改动 AGENTS.md / CLAUDE.md / .claude/rules 等 Cadence-skills 文件。确认彻底删除？",
    );
    if (!confirmed) return;
    setError(null);
    try {
      await purgeLogicalCodebase(selectedProjectId, logicalCodebaseId);
      await refresh(selectedProjectId);
    } catch (reason) {
      setError(errorMessage(reason, "彻底删除逻辑代码库失败"));
    }
  }

  return {
    handlePublishFull: async () => {
      await handlePublish("full");
    },
    handlePublishIncremental: async () => {
      await handlePublish("incremental");
    },
    handleRetryRepo,
    handleRevokePublication,
    handleRebuildAggregateIndex,
    handleStartAggregateInitialization,
    handleCancelAggregateInitialization,
    handleReinitializeAggregateInitialization,
    handleDeleteProject,
    handleDeleteRepository,
    handleDeleteLogicalCodebase,
    handleRestoreLogicalCodebase,
    handlePurgeLogicalCodebase,
  };
}
