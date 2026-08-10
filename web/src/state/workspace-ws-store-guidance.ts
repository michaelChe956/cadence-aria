import type { WorkspaceProviderName } from "../api/types";
import type { WsMessage } from "./workspace-ws-store-types";

const PROVIDER_INTERACTION_GUIDANCE: Record<WorkspaceProviderName, string> = {
  claude_code:
    "当前 author provider 是 Claude Code；需要向用户确认时，必须使用结构化 AskUserQuestion，让同一个 Claude Code 进程等待用户回答后继续。禁止输出文本 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。",
  codex:
    "当前 author provider 是 Codex；需要向用户确认时，必须使用结构化 requestUserInput，让同一个 Codex turn 等待用户回答后继续。禁止输出文本 1/2/3 或 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。",
  pi:
    "当前 author provider 是 Pi；Pi 不声明原生结构化交互能力。需要向用户确认时，必须输出 daemon 可识别的暂停信号并交给 text_fallback。禁止伪造 AskUserQuestion 或 requestUserInput 工具调用，也不要把文本选择题作为正常交互路径。",
  kimi_code:
    "当前 author provider 是 Kimi Code；需要向用户确认时，必须使用结构化 AskUserQuestion 并等待回答。禁止输出文本 A/B/C 选择题作为交互替代。",
  fake:
    "当前 author provider 未声明原生结构化交互能力；需要向用户确认时，必须输出 daemon 可识别的暂停信号并交给 text_fallback。禁止伪造 AskUserQuestion 或 requestUserInput 工具调用，也不要把文本选择题作为正常交互路径。",
};

export function refreshPreparedContextAuthorGuidance(
  messages: WsMessage[],
  provider: WorkspaceProviderName,
) {
  let changed = false;
  const nextMessages = messages.map((message) => {
    const content = refreshPreparedContextAuthorGuidanceContent(message.content, provider);
    if (content === message.content) {
      return message;
    }
    changed = true;
    return { ...message, content };
  });
  return changed ? nextMessages : messages;
}

function refreshPreparedContextAuthorGuidanceContent(
  content: string,
  provider: WorkspaceProviderName,
) {
  if (!content.startsWith("Workspace 生成任务已准备")) {
    return content;
  }
  const marker = "\n[workflow_discipline]\n";
  const sectionStart = content.indexOf(marker);
  if (sectionStart === -1) {
    return content;
  }
  const disciplineStart = sectionStart + marker.length;
  const sectionEnd = content.indexOf("\n\n[", disciplineStart);
  const safeSectionEnd = sectionEnd === -1 ? content.length : sectionEnd;
  const section = content.slice(disciplineStart, safeSectionEnd);
  const guidanceStart = section.lastIndexOf("\n当前 author provider");
  if (guidanceStart === -1) {
    return content;
  }
  const nextSection =
    section.slice(0, guidanceStart) + "\n" + PROVIDER_INTERACTION_GUIDANCE[provider];
  return content.slice(0, disciplineStart) + nextSection + content.slice(safeSectionEnd);
}
