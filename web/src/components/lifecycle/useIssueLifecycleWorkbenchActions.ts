import type { Dispatch, SetStateAction } from "react";
import { ApiRequestError, deleteProject, deleteRepository } from "../../api/client";
import { rebuildAggregateIndex } from "../../api/aggregate-index";
import { cancelAggregateInitialization, startAggregateInitialization } from "../../api/aggregate-initialization";
import { deleteLogicalCodebase } from "../../api/codebases";
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
  setSelectedProjectId: Dispatch<SetStateAction<string | null>>;
  setFocusedIssueId: Dispatch<SetStateAction<string | null>>;
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
  aggregateInitialization, selectedLogicalCodebaseId, setSelectedProjectId,
  setFocusedIssueId, setSelectedCardKey, setSelectedLogicalCodebaseId, setError,
  setPointerPublicationBusy, setPointerPublications, setAggregateIndex, setAggregateIndexRebuilding,
  setAggregateInitialization, setAggregateInitializationBusy, refresh,
}: Args) {
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

  async function handleRebuildAggregateIndex() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setError("缺少 Project 或逻辑代码库");
      return;
    }
    setAggregateIndexRebuilding(true);
    setError(null);
    try {
      setAggregateIndex(await rebuildAggregateIndex(selectedProjectId, activeLogicalCodebaseId));
    } catch (reason) {
      setError(reason instanceof ApiRequestError ? reason.message : errorMessage(reason, "重建聚合索引失败"));
    } finally {
      setAggregateIndexRebuilding(false);
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

  async function handleDeleteProject(projectId: string) {
    setError(null);
    await deleteProject(projectId);
    if (projectId === selectedProjectId) {
      setSelectedProjectId(null);
      setFocusedIssueId(null);
      setSelectedCardKey(null);
    }
    await refresh(null);
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
    const confirmed = window.confirm("删除逻辑代码库将软删除其登记成员、聚合索引与指针发布上下文，且无法在界面内撤销。确认删除？");
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
    handleDeleteProject,
    handleDeleteRepository,
    handleDeleteLogicalCodebase,
  };
}
