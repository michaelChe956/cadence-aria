import { create } from "zustand";

export interface CodingLogLine {
  id: number;
  nodeId: string | null;
  nodeTitle: string | null;
  text: string;
  at: string;
  kind: "stream" | "event";
}

export type CodingLogEntryInput = Omit<CodingLogLine, "id">;

export const CODING_LOG_TAIL_LIMIT = 500;

interface CodingLogState {
  lines: CodingLogLine[];
  nextId: number;
  appendLines: (entries: readonly CodingLogEntryInput[]) => void;
  reset: () => void;
}

// 渲染解耦（REQ-UI37-15 第 4 项 / REQ-UI37-13「日志抽取与对话流渲染解耦」）：
// 本 store 与 useCodingWorkspaceStore 完全分离，日志追加不改变对话 store 的任何引用，
// 对话流列表（订阅 coding store）不因日志更新而重渲。
export const useCodingLogStore = create<CodingLogState>((set) => ({
  lines: [],
  nextId: 1,
  appendLines: (entries) => {
    if (entries.length === 0) return;
    set((state) => {
      let nextId = state.nextId;
      const appended = entries.map((line) => ({ ...line, id: nextId++ }));
      const lines = [...state.lines, ...appended];
      return {
        lines: lines.length > CODING_LOG_TAIL_LIMIT ? lines.slice(lines.length - CODING_LOG_TAIL_LIMIT) : lines,
        nextId,
      };
    });
  },
  reset: () => set({ lines: [], nextId: 1 }),
}));

export function filterCodingLogLines(
  lines: readonly CodingLogLine[],
  nodeId: string | null,
): readonly CodingLogLine[] {
  if (nodeId === null) return lines;
  return lines.filter((line) => line.nodeId === nodeId);
}

export function codingLogNodeOptions(
  lines: readonly CodingLogLine[],
): { nodeId: string; nodeTitle: string }[] {
  const seen = new Set<string>();
  const options: { nodeId: string; nodeTitle: string }[] = [];
  for (const line of lines) {
    if (line.nodeId === null || seen.has(line.nodeId)) continue;
    seen.add(line.nodeId);
    options.push({ nodeId: line.nodeId, nodeTitle: line.nodeTitle ?? line.nodeId });
  }
  return options;
}
