import { useCallback, useEffect, useRef, useState } from "react";
import { getIssueLifecycle } from "../../api/client";
import type { IssueLifecycleResponse, ProductIssue } from "../../api/types";
import { subscribeToLifecycleInvalidation } from "../../state/lifecycle-workbench-store";
import { errorMessage, normalizeLifecycleResponse } from "./IssueLifecycleWorkbenchParts";

const DETAIL_CONCURRENCY = 3;

type DetailBatch = {
  projectId: string;
  issues: ReadonlyMap<string, ProductIssue>;
  pending: Set<string>;
  invalidated: Set<string>;
  revisions: Map<string, number>;
  focusedIssueId: string | null;
  drawerIssueId: string | undefined;
};

export function useWorkbenchLifecycleDetails() {
  const [details, setDetails] = useState<ReadonlyMap<string, IssueLifecycleResponse>>(
    () => new Map(),
  );
  const [detailErrors, setDetailErrors] = useState<ReadonlyMap<string, string>>(
    () => new Map(),
  );
  const batchRef = useRef<DetailBatch | null>(null);
  const activeReads = useRef(0);

  const drain = useCallback(() => {
    const batch = batchRef.current;
    if (!batch) return;

    while (activeReads.current < DETAIL_CONCURRENCY && batch.pending.size > 0) {
      const issueId = [batch.focusedIssueId, batch.drawerIssueId].find(
        (id): id is string => id != null && batch.pending.has(id),
      ) ?? batch.invalidated.values().next().value ?? batch.pending.values().next().value;
      if (issueId === undefined) break;
      batch.pending.delete(issueId);
      batch.invalidated.delete(issueId);
      const issue = batch.issues.get(issueId)!;
      const revision = batch.revisions.get(issueId);
      activeReads.current += 1;

      void getIssueLifecycle(issueId, batch.projectId)
        .then((response) => {
          if (batchRef.current !== batch || batch.revisions.get(issueId) !== revision) return;
          const normalized = normalizeLifecycleResponse(response, issue);
          if (normalized.issue.project_id !== batch.projectId) {
            throw new Error("invalid lifecycle response");
          }
          setDetails((previous) => new Map(previous).set(issueId, normalized));
          setDetailErrors((previous) => {
            if (!previous.has(issueId)) return previous;
            const next = new Map(previous);
            next.delete(issueId);
            return next;
          });
        })
        .catch((reason: unknown) => {
          if (batchRef.current !== batch || batch.revisions.get(issueId) !== revision) return;
          const message = errorMessage(reason, "").trim() || "读取 Issue 生命周期失败";
          setDetailErrors((previous) => new Map(previous).set(issueId, message));
        })
        .finally(() => {
          activeReads.current -= 1;
          drain();
        });
    }
  }, []);

  function clearDetails() {
    batchRef.current = null;
    setDetails(new Map());
    setDetailErrors(new Map());
  }

  function hydrateDetails(
    projectId: string,
    issues: readonly ProductIssue[],
    focusedIssueId: string | null,
    drawerIssueId: string | undefined,
  ) {
    const issuesById = new Map(issues.map((issue) => [issue.issue_id, issue]));
    const batch: DetailBatch = {
      projectId,
      issues: issuesById,
      pending: new Set(issuesById.keys()),
      invalidated: new Set(),
      revisions: new Map(),
      focusedIssueId,
      drawerIssueId,
    };
    // Outstanding reads keep their slots on refresh or project switch. Only
    // queued work and late responses from the old batch are discarded.
    batchRef.current = batch;
    setDetailErrors(new Map());
    setDetails((previous) => new Map([...previous].filter(([id, detail]) =>
      batch.issues.has(id) && detail.issue.project_id === projectId,
    )));

    drain();
  }

  const prioritizeDetails = useCallback((focusedIssueId: string | null, drawerIssueId?: string) => {
    const batch = batchRef.current;
    if (!batch) return;
    batch.focusedIssueId = focusedIssueId;
    batch.drawerIssueId = drawerIssueId;
    drain();
  }, [drain]);

  const invalidateIssue = useCallback((batch: DetailBatch, issueId: string) => {
    batch.revisions.set(issueId, (batch.revisions.get(issueId) ?? 0) + 1);
    batch.pending.add(issueId);
    batch.invalidated.add(issueId);
    drain();
  }, [drain]);

  function refreshDeletedIssueDetails(projectId: string, issueId: string) {
    const batch = batchRef.current;
    if (batch?.projectId !== projectId || !batch.issues.has(issueId)) return;
    // A deletion can cascade, so none of the previous issue detail is authoritative.
    setDetails((previous) => {
      const next = new Map(previous);
      next.delete(issueId);
      return next;
    });
    setDetailErrors((previous) => {
      if (!previous.has(issueId)) return previous;
      const next = new Map(previous);
      next.delete(issueId);
      return next;
    });
    invalidateIssue(batch, issueId);
  }

  useEffect(() => {
    const unsubscribe = subscribeToLifecycleInvalidation(({ issueId }) => {
      const batch = batchRef.current;
      if (!batch?.issues.has(issueId)) return;
      invalidateIssue(batch, issueId);
    });
    return () => {
      unsubscribe();
      batchRef.current = null;
    };
  }, [invalidateIssue]);

  return { details, detailErrors, clearDetails, hydrateDetails, prioritizeDetails, refreshDeletedIssueDetails };
}
