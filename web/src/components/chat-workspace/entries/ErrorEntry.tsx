import { TriangleAlert } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function ErrorEntry({ entry }: { entry: ChatEntry }) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const code = typeof metadata?.code === "string" ? metadata.code : null;

  return (
    <ChatEntryContainer
      role="system"
      title={code ? `错误 ${code}` : "错误"}
      className="border-red-200 bg-red-50 text-red-800"
    >
      <div className="flex items-start gap-2 text-sm">
        <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-red-600" />
        <div className="min-w-0 whitespace-pre-wrap">{entry.content}</div>
      </div>
    </ChatEntryContainer>
  );
}
