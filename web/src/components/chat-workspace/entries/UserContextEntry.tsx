import { FileText, User } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function UserContextEntry({ entry }: { entry: ChatEntry }) {
  const isPrepared = entry.metadata?.prepared === true;
  const isProviderPrompt = entry.metadata?.prompt_source === "provider_prompt";
  const provider = typeof entry.metadata?.provider === "string" ? entry.metadata.provider : null;
  const promptNodeTitle =
    typeof entry.metadata?.prompt_node_title === "string" ? entry.metadata.prompt_node_title : null;
  const title = isProviderPrompt ? "实际执行 Prompt" : isPrepared ? "初始化输入" : "你";
  const ContextIcon = isProviderPrompt ? FileText : User;
  const sourceLabel = isProviderPrompt ? "PROMPT" : "CONTEXT";
  const sourceTitle =
    promptNodeTitle ?? (isProviderPrompt ? "Provider 调用输入" : "Workspace 初始化上下文");
  const lineCount = countLines(entry.content);
  const characterCount = entry.content.length;
  const labelTone = isProviderPrompt
    ? "border-blue-200 bg-blue-50 text-blue-700"
    : "border-emerald-200 bg-emerald-50 text-emerald-700";
  const iconTone = isProviderPrompt
    ? "border-blue-200 bg-blue-50 text-blue-700"
    : "border-emerald-200 bg-emerald-50 text-emerald-700";

  return (
    <ChatEntryContainer
      role="user"
      title={title}
      wide={isPrepared || isProviderPrompt}
      className={isPrepared || isProviderPrompt ? "border-slate-200 bg-white" : "border-blue-200"}
    >
      {isPrepared || isProviderPrompt ? (
        <div className="overflow-hidden rounded-md border border-[var(--aria-line)] bg-white">
          <div className="flex min-w-0 flex-col gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex min-w-0 items-center gap-2">
              <span
                className={[
                  "flex h-7 w-7 shrink-0 items-center justify-center rounded border",
                  iconTone,
                ].join(" ")}
              >
                <ContextIcon className="h-3.5 w-3.5" />
              </span>
              <div className="min-w-0">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <span
                    className={[
                      "shrink-0 rounded border px-1.5 py-0.5 font-mono text-[10px] font-semibold",
                      labelTone,
                    ].join(" ")}
                  >
                    {sourceLabel}
                  </span>
                  <span className="min-w-0 truncate text-xs font-semibold text-[var(--aria-ink)]">
                    {sourceTitle}
                  </span>
                </div>
                <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 font-mono text-[10px] text-[var(--aria-ink-muted)]">
                  <span>{lineCount.toLocaleString()} 行</span>
                  <span aria-hidden="true">/</span>
                  <span>{characterCount.toLocaleString()} 字符</span>
                </div>
              </div>
            </div>
            {provider ? (
              <span className="w-fit shrink-0 rounded border border-[var(--aria-line)] bg-white px-2 py-1 font-mono text-[10px] font-semibold uppercase text-[var(--aria-ink-muted)]">
                {provider}
              </span>
            ) : null}
          </div>
          <pre
            className={[
              "overflow-auto whitespace-pre-wrap break-words bg-white px-3 py-3 font-mono text-[11px] leading-5 text-[var(--aria-ink)] [overflow-wrap:anywhere] sm:text-xs",
              isProviderPrompt ? "max-h-[32rem]" : "max-h-96",
            ].join(" ")}
          >
            {entry.content}
          </pre>
        </div>
      ) : (
        <div className="whitespace-pre-wrap text-sm text-[var(--aria-ink)]">{entry.content}</div>
      )}
    </ChatEntryContainer>
  );
}

function countLines(content: string) {
  if (content.length === 0) {
    return 0;
  }
  return content.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n").length;
}
