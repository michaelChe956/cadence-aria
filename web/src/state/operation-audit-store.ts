import { create } from "zustand";
import {
  OPERATOR_LABEL,
  type OperationAuditRecord,
} from "./cockpit-operation-semantics";

export { OPERATOR_LABEL } from "./cockpit-operation-semantics";

export interface TakeoverLink {
  parentSessionId: string;
  takeoverEventId: string;
}

export interface OperationAuditStore {
  records: readonly OperationAuditRecord[];
  takeoverLinks: Readonly<Record<string, TakeoverLink>>;
  record(input: Omit<OperationAuditRecord, "id" | "atMs" | "atIso" | "operator">): string;
  markRejected(recordId: string, code: string): void;
  markCompleted(recordId: string): void;
  recordTakeoverLink(
    childSessionId: string,
    parentSessionId: string,
    takeoverEventId: string,
  ): void;
  reset(): void;
}

export const useOperationAuditStore = create<OperationAuditStore>((set) => ({
  records: [],
  takeoverLinks: {},
  record: (input) => {
    const atMs = Date.now();
    const record: OperationAuditRecord = {
      ...input,
      id: crypto.randomUUID(),
      atMs,
      atIso: new Date(atMs).toISOString(),
      operator: OPERATOR_LABEL,
    };
    set((state) => ({ records: [...state.records, record].slice(-100) }));
    return record.id;
  },
  markRejected: (recordId, code) =>
    set((state) => ({
      records: state.records.map((record) =>
        record.id === recordId ? { ...record, outcome: "rejected", detail: code } : record,
      ),
    })),
  markCompleted: (recordId) =>
    set((state) => ({
      records: state.records.map((record) =>
        record.id === recordId ? { ...record, outcome: "completed", detail: null } : record,
      ),
    })),
  recordTakeoverLink: (childSessionId, parentSessionId, takeoverEventId) =>
    set((state) => ({
      takeoverLinks: {
        ...state.takeoverLinks,
        [childSessionId]: { parentSessionId, takeoverEventId },
      },
    })),
  reset: () => set({ records: [], takeoverLinks: {} }),
}));
