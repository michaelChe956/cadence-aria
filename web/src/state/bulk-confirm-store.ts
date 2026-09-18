import { create } from "zustand";
import {
  BULK_CONFIRM_CONCURRENCY,
  createBulkSocketFactory,
  runBulkConfirm,
  type BulkConfirmItemOutcome,
  type BulkConfirmTarget,
  type BulkSocketFactory,
} from "./bulk-confirm-runner";

export interface BulkConfirmRunState {
  id: string;
  startedAtIso: string;
  concurrency: number;
  items: readonly BulkConfirmItemOutcome[];
}

interface BulkConfirmStore {
  runs: readonly BulkConfirmRunState[];
  busy: boolean;
  start(targets: readonly BulkConfirmTarget[], socketFactory?: BulkSocketFactory): void;
  reset(): void;
}

const patchItem = (
  items: readonly BulkConfirmItemOutcome[],
  outcome: BulkConfirmItemOutcome,
) => items.map((item) => (item.itemId === outcome.itemId ? outcome : item));

export const useBulkConfirmStore = create<BulkConfirmStore>((set, get) => ({
  runs: [],
  busy: false,
  start: (targets, socketFactory) => {
    if (get().busy || targets.length === 0) {
      return;
    }
    const run: BulkConfirmRunState = {
      id: crypto.randomUUID(),
      startedAtIso: new Date().toISOString(),
      concurrency: BULK_CONFIRM_CONCURRENCY,
      items: targets.map((target) => ({
        ...target,
        status: "pending" as const,
        detail: null,
        atIso: new Date().toISOString(),
      })),
    };
    set((state) => ({ busy: true, runs: [...state.runs, run].slice(-5) }));
    void runBulkConfirm(targets, socketFactory ?? createBulkSocketFactory(), (outcome) => {
      set((state) => ({
        runs: state.runs.map((candidate) =>
          candidate.id === run.id
            ? { ...candidate, items: patchItem(candidate.items, outcome) }
            : candidate,
        ),
      }));
    }).finally(() => set({ busy: false }));
  },
  reset: () => set({ runs: [], busy: false }),
}));
