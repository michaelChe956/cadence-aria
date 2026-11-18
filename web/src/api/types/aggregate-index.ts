import type { FailureCopyDto } from "./logical-codebase-bootstrap";

export type AggregateIndexState =
  | "active"
  | "stale"
  | "degraded"
  | "rebuilding"
  | "missing";

export type AggregateIndexActiveResponse = {
  state: AggregateIndexState;
  revision?: number | null;
  indexed_at?: string | null;
  warning?: string | null;
  /** 索引不可用时的中文成品（问题/原因/怎么做/机器码）；索引可用时缺省。 */
  evidence_copy?: FailureCopyDto | null;
  /** 最新一代聚合索引失败时的中文成品（问题/原因/怎么做/机器码）；无失败时缺省。 */
  failure_copy?: FailureCopyDto | null;
};
