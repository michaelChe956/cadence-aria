import { useEffect, useState } from "react";
import { getLegacyCodingAttemptSnapshot } from "../api/client";
import type { CodingAttemptAddress } from "../api/types";

export function LegacyCodingWorkspaceRedirect({
  attemptId,
  onResolved,
  onBack,
}: {
  attemptId: string;
  onResolved: (address: CodingAttemptAddress) => void;
  onBack: () => void;
}) {
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);

    getLegacyCodingAttemptSnapshot(attemptId)
      .then((snapshot) => {
        if (cancelled) return;
        onResolved({
          projectId: snapshot.attempt.project_id,
          issueId: snapshot.attempt.issue_id,
          attemptId: snapshot.attempt.attempt_id,
        });
      })
      .catch((reason: { code?: string }) => {
        if (cancelled) return;
        setError(
          reason.code === "coding_attempt_ambiguous"
            ? "该历史 Coding Attempt ID 对应多个 Issue，请从目标 Issue 的 Workbench 重新进入。"
            : "Coding Attempt 不存在或已删除。",
        );
      });

    return () => {
      cancelled = true;
    };
  }, [attemptId, onResolved]);

  if (!error) {
    return <div role="status">正在定位 Coding Attempt…</div>;
  }

  return (
    <div role="alert">
      <p>{error}</p>
      <button type="button" onClick={onBack}>
        返回 Workbench
      </button>
    </div>
  );
}
