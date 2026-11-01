// REQ-UI37-16 第 1 项：轮次间 markdown diff 由前端自算（禁新依赖）。
// 行级 LCS：计划文档为百行级 markdown，2000 行上限内 DP 表 ~16MB Int32，足够；
// 超限降级为整档替换并置 truncated，界面必须明示（Global Constraints 第 6 条）。

export type RevisionDiffLineKind = "same" | "added" | "removed";

export interface RevisionDiffLine {
  kind: RevisionDiffLineKind;
  text: string;
  oldLineNo: number | null;
  newLineNo: number | null;
}

export interface RevisionDiffSummary {
  added: number;
  removed: number;
  changed: number;
}

export interface RevisionDiff {
  lines: RevisionDiffLine[];
  summary: RevisionDiffSummary;
  truncated: boolean;
}

export interface RevisionDiffHunk {
  header: string;
  lines: RevisionDiffLine[];
}

export const REVISION_DIFF_MAX_LINES = 2000;
export const REVISION_DIFF_CONTEXT_LINES = 3;

export function computeRevisionDiff(
  oldMarkdown: string,
  newMarkdown: string,
): RevisionDiff {
  const oldLines = splitMarkdownLines(oldMarkdown);
  const newLines = splitMarkdownLines(newMarkdown);
  const truncated =
    oldLines.length > REVISION_DIFF_MAX_LINES ||
    newLines.length > REVISION_DIFF_MAX_LINES;

  if (truncated) {
    // 降级语义是整档替换：诚实摘要即全删+全增、changed=0，
    // 不走 summarizeChanges 的同区段配对口径（会把整档替换误报为部分 changed）。
    return {
      lines: wholeReplacement(oldLines, newLines),
      summary: { added: newLines.length, removed: oldLines.length, changed: 0 },
      truncated,
    };
  }

  const lines = lcsDiffLines(oldLines, newLines);
  return { lines, summary: summarizeChanges(lines), truncated };
}

export function toUnifiedHunks(
  lines: readonly RevisionDiffLine[],
  contextLines: number = REVISION_DIFF_CONTEXT_LINES,
): RevisionDiffHunk[] {
  const changedIndexes: number[] = [];
  lines.forEach((line, index) => {
    if (line.kind !== "same") {
      changedIndexes.push(index);
    }
  });
  if (changedIndexes.length === 0) {
    return [];
  }

  const groups: Array<[number, number]> = [];
  let groupStart = changedIndexes[0];
  let previous = changedIndexes[0];
  for (const index of changedIndexes.slice(1)) {
    if (index - previous > contextLines * 2 + 1) {
      groups.push([groupStart, previous]);
      groupStart = index;
    }
    previous = index;
  }
  groups.push([groupStart, previous]);

  return groups.map(([start, end]) => {
    const sliceStart = Math.max(0, start - contextLines);
    const sliceEnd = Math.min(lines.length - 1, end + contextLines);
    const slice = lines.slice(sliceStart, sliceEnd + 1);
    return {
      header: hunkHeaderFor(lines, sliceStart, slice),
      lines: slice,
    };
  });
}

function splitMarkdownLines(markdown: string): string[] {
  const normalized = markdown.replace(/\r\n?/g, "\n");
  if (normalized === "") {
    return [];
  }
  const lines = normalized.split("\n");
  if (lines.at(-1) === "") {
    lines.pop();
  }
  return lines;
}

function wholeReplacement(
  oldLines: string[],
  newLines: string[],
): RevisionDiffLine[] {
  return [
    ...oldLines.map<RevisionDiffLine>((text, index) => ({
      kind: "removed",
      text,
      oldLineNo: index + 1,
      newLineNo: null,
    })),
    ...newLines.map<RevisionDiffLine>((text, index) => ({
      kind: "added",
      text,
      oldLineNo: null,
      newLineNo: index + 1,
    })),
  ];
}

function lcsDiffLines(
  oldLines: string[],
  newLines: string[],
): RevisionDiffLine[] {
  const width = newLines.length + 1;
  const table = lcsTable(oldLines, newLines, width);
  const lines: RevisionDiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < oldLines.length && j < newLines.length) {
    if (oldLines[i] === newLines[j]) {
      lines.push({
        kind: "same",
        text: oldLines[i],
        oldLineNo: i + 1,
        newLineNo: j + 1,
      });
      i += 1;
      j += 1;
      continue;
    }
    // 平局取上移（先输出 removed）：swap 用例依赖该确定性。
    if (table[(i + 1) * width + j] >= table[i * width + j + 1]) {
      lines.push({
        kind: "removed",
        text: oldLines[i],
        oldLineNo: i + 1,
        newLineNo: null,
      });
      i += 1;
    } else {
      lines.push({
        kind: "added",
        text: newLines[j],
        oldLineNo: null,
        newLineNo: j + 1,
      });
      j += 1;
    }
  }
  while (i < oldLines.length) {
    lines.push({
      kind: "removed",
      text: oldLines[i],
      oldLineNo: i + 1,
      newLineNo: null,
    });
    i += 1;
  }
  while (j < newLines.length) {
    lines.push({
      kind: "added",
      text: newLines[j],
      oldLineNo: null,
      newLineNo: j + 1,
    });
    j += 1;
  }
  return lines;
}

function lcsTable(oldLines: string[], newLines: string[], width: number): Int32Array {
  const table = new Int32Array((oldLines.length + 1) * width);
  for (let i = oldLines.length - 1; i >= 0; i -= 1) {
    for (let j = newLines.length - 1; j >= 0; j -= 1) {
      table[i * width + j] =
        oldLines[i] === newLines[j]
          ? table[(i + 1) * width + j + 1] + 1
          : Math.max(table[(i + 1) * width + j], table[i * width + j + 1]);
    }
  }
  return table;
}

// 摘要口径：连续非 same 区段内，removed 与 added 按 min 配对计 changed，
// 余额分别计入 removed / added——与「这一轮改了几行、净增删几行」直觉一致。
function summarizeChanges(lines: readonly RevisionDiffLine[]): RevisionDiffSummary {
  const summary: RevisionDiffSummary = { added: 0, removed: 0, changed: 0 };
  let index = 0;
  while (index < lines.length) {
    if (lines[index].kind === "same") {
      index += 1;
      continue;
    }
    let removedInRun = 0;
    let addedInRun = 0;
    while (index < lines.length && lines[index].kind !== "same") {
      if (lines[index].kind === "removed") {
        removedInRun += 1;
      } else {
        addedInRun += 1;
      }
      index += 1;
    }
    const paired = Math.min(removedInRun, addedInRun);
    summary.changed += paired;
    summary.removed += removedInRun - paired;
    summary.added += addedInRun - paired;
  }
  return summary;
}

function hunkHeaderFor(
  lines: readonly RevisionDiffLine[],
  sliceStart: number,
  slice: readonly RevisionDiffLine[],
): string {
  const oldCount = slice.filter((line) => line.oldLineNo !== null).length;
  const newCount = slice.filter((line) => line.newLineNo !== null).length;
  const oldStart = boundaryLineNo(lines, sliceStart, slice, "oldLineNo");
  const newStart = boundaryLineNo(lines, sliceStart, slice, "newLineNo");
  return `@@ -${oldStart},${oldCount} +${newStart},${newCount} @@`;
}

function boundaryLineNo(
  lines: readonly RevisionDiffLine[],
  sliceStart: number,
  slice: readonly RevisionDiffLine[],
  key: "oldLineNo" | "newLineNo",
): number {
  // 切片内首个带该侧行号的行即该侧起点（count>0 时必存在）。
  const firstNumbered = slice.find((line) => line[key] !== null);
  if (firstNumbered) {
    return firstNumbered[key] as number;
  }
  // 计数为 0 的一侧（纯新增的 old 侧 / 纯删除的 new 侧）：定位到切片前一行之后；
  // 文件开头之前无数行 → git 惯例 0。
  for (let index = sliceStart - 1; index >= 0; index -= 1) {
    const lineNo = lines[index][key];
    if (lineNo !== null) {
      return lineNo + 1;
    }
  }
  return 0;
}
