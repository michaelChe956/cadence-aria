import { useCallback, useEffect, useRef, useState } from "react";
import { getIssueLifecycle } from "../../api/client";
import type { IssueLifecycleResponse, ProductIssue } from "../../api/types";
import { subscribeToLifecycleInvalidation } from "../../state/lifecycle-workbench-store";
import { errorMessage, normalizeLifecycleResponse } from "./IssueLifecycleWorkbenchParts";

const DETAIL_CONCURRENCY = 3;

type DetailBatch = {
  projectId: string;
  issues: ReadonlyMap<string, ProductIssue>;
  reads: Map<string, Promise<void>>;
};

export function useWorkbenchLifecycleDetails(onError: (message: string) => void) {
  const [details, setDetails] = useState<ReadonlyMap<string, IssueLifecycleResponse>>(
    () => new Map(),
  );
  const [failed, setFailed] = useState<ReadonlySet<string>>(() => new Set());
  const batchRef = useRef<DetailBatch | null>(null);

  const readIssue = useCallback((issueId: string, invalidate = false) => {
    const batch = batchRef.current;
    const issue = batch?.issues.get(issueId);
    if (!batch || !issue) return;

    const existing = batch.reads.get(issueId);
    if (existing && !invalidate) return existing;
    if (invalidate) {
      setFailed((previous) => {
        if (!previous.has(issueId)) return previous;
        const next = new Set(previous);
        next.delete(issueId);
        return next;
      });
    }

    // Invalidation supersedes only this issue, not the rest of the queue.
    const read = getIssueLifecycle(issueId, batch.projectId)
      .then((response) => {
        if (batchRef.current !== batch || batch.reads.get(issueId) !== read) return;
        const normalized = normalizeLifecycleResponse(response, issue);
        setDetails((previous) => new Map(previous).set(issueId, normalized));
      })
      .catch((reason: unknown) => {
        if (batchRef.current !== batch || batch.reads.get(issueId) !== read) return;
        setFailed((previous) => new Set(previous).add(issueId));
        onError(errorMessage(reason, "读取 Issue 生命周期失败"));
      });
    batch.reads.set(issueId, read);
    return read;
  }, [onError]);

  function clearDetails() {
    batchRef.current = null;
    setDetails(new Map());
    setFailed(new Set());
  }

  async function hydrateDetails(
    projectId: string,
    issues: readonly ProductIssue[],
    focusedIssueId: string | null,
    drawerIssueId: string | undefined,
  ) {
    const batch: DetailBatch = {
      projectId,
      issues: new Map(issues.map((issue) => [issue.issue_id, issue])),
      reads: new Map(),
    };
    batchRef.current = batch;
    setFailed(new Set());
    setDetails((previous) => new Map([...previous].filter(([id]) => batch.issues.has(id))));

    // The focused issue and a linked drawer go first. Each slot then fills the
    // queue in order, without waiting for another slot's slow request.
    const order = new Set([focusedIssueId, drawerIssueId, ...batch.issues.keys()]);
    order.delete(null);
    order.delete(undefined);
    const remaining = order.values();
    const fillQueue = async () => {
      for (const issueId of remaining) {
        if (batchRef.current !== batch) return;
        if (issueId) await readIssue(issueId);
      }
    };
    await Promise.all(Array.from({ length: Math.min(DETAIL_CONCURRENCY, order.size) }, fillQueue));
  }

  useEffect(() => {
    const unsubscribe = subscribeToLifecycleInvalidation(({ issueId }) => {
      void readIssue(issueId, true);
    });
    return () => {
      unsubscribe();
      batchRef.current = null;
    };
  }, [readIssue]);

  return { details, failed, clearDetails, hydrateDetails, readIssue };
}
