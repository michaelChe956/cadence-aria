export const CHAT_COCKPIT_STORAGE_KEY = "aria.chat.cockpit";

export type ChatCockpitMode = "legacy" | "cockpit";

// 显式设置始终优先；未设置时由会话类型选择默认形态。
export function readChatCockpitMode(
  workspaceType: string | null,
): ChatCockpitMode {
  try {
    const stored = window.localStorage.getItem(CHAT_COCKPIT_STORAGE_KEY);
    if (stored === "cockpit" || stored === "legacy") {
      return stored;
    }
    return workspaceType === "work_item_plan" ? "cockpit" : "legacy";
  } catch {
    return "legacy";
  }
}

export function writeChatCockpitMode(mode: ChatCockpitMode): void {
  try {
    window.localStorage.setItem(CHAT_COCKPIT_STORAGE_KEY, mode);
  } catch {
    // 存储不可用时静默降级为本次会话内存态之外的默认形态（legacy），不抛错、不影响渲染。
  }
}
