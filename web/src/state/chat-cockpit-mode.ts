export const CHAT_COCKPIT_STORAGE_KEY = "aria.chat.cockpit";

export type ChatCockpitMode = "legacy" | "cockpit";

// M5：默认 legacy（旧形态）。灰度 = parity 达标后只翻这里的默认值，不动路由、不新增开关组件。
export function readChatCockpitMode(): ChatCockpitMode {
  try {
    const stored = window.localStorage.getItem(CHAT_COCKPIT_STORAGE_KEY);
    return stored === "cockpit" ? "cockpit" : "legacy";
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
