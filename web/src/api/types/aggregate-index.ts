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
};
