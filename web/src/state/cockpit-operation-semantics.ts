export const OPERATOR_LABEL = "本地浏览器" as const;
export const DANGEROUS_CONFIRM_TIMEOUT_MS = 10_000;

export type CockpitOperation =
  | "confirm"
  | "feedback"
  | "request_change"
  | "terminate"
  | "takeover"
  | "advance"
  | "start_coding"
  | "final_confirm"
  | "stage_gate_confirm"
  | "confirm_plan_amendment";

export type CockpitHotkeyAction = "confirm" | "feedback" | "takeover" | "advance";

export interface CockpitHotkeyHandlers {
  confirm(): void;
  feedback(): void;
  takeover(): void;
  advance(): void;
}

export const COCKPIT_HOTKEYS: Readonly<Record<CockpitHotkeyAction, {
  code: "Enter" | "KeyF" | "KeyT" | "KeyA";
  ctrlOrMeta: boolean;
  shift: boolean;
}>> = {
  confirm: { code: "Enter", ctrlOrMeta: true, shift: false },
  feedback: { code: "KeyF", ctrlOrMeta: true, shift: false },
  takeover: { code: "KeyT", ctrlOrMeta: true, shift: true },
  advance: { code: "KeyA", ctrlOrMeta: true, shift: false },
};

export const BULK_OPERATION_WHITELIST: ReadonlySet<CockpitOperation> = new Set(["confirm"]);

export function canBulkApply(operation: CockpitOperation): boolean {
  return BULK_OPERATION_WHITELIST.has(operation);
}

export function isTextEditingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (
    target.isContentEditable ||
    target.tagName === "INPUT" ||
    target.tagName === "TEXTAREA" ||
    target.tagName === "SELECT"
  );
}

export interface ConfirmTwiceButtonHandle {
  /** 未 armed 时进入确认态；已 armed 时执行同一组件的 onConfirm 并复位。 */
  arm(): void;
}

export interface OperationAuditRecord {
  id: string;
  atMs: number | null;
  atIso: string | null;
  operator: typeof OPERATOR_LABEL;
  sessionId: string;
  gateId: string | null;
  operation: CockpitOperation;
  source: "chat" | "coding" | "takeover" | "system_recovery";
  outcome: "sent" | "rejected" | "completed";
  detail: string | null;
}
