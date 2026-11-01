import { describe, expect, it } from "vitest";
import {
  REVISION_DIFF_MAX_LINES,
  computeRevisionDiff,
  toUnifiedHunks,
} from "./revision-diff";

describe("computeRevisionDiff", () => {
  it("returns all-same lines with a zero summary for identical markdown", () => {
    const diff = computeRevisionDiff("# 计划\n## 目标\n", "# 计划\n## 目标\n");
    expect(diff.truncated).toBe(false);
    expect(diff.summary).toEqual({ added: 0, removed: 0, changed: 0 });
    expect(diff.lines.map((line) => line.kind)).toEqual(["same", "same"]);
    expect(diff.lines[0]).toMatchObject({
      text: "# 计划",
      oldLineNo: 1,
      newLineNo: 1,
    });
  });

  it("normalizes CRLF before comparing", () => {
    const diff = computeRevisionDiff("# 计划\r\n## 目标\r\n", "# 计划\n## 目标\n");
    expect(diff.summary).toEqual({ added: 0, removed: 0, changed: 0 });
  });

  it("counts a replaced line as changed, not added plus removed", () => {
    const diff = computeRevisionDiff("# 计划\n旧目标\n", "# 计划\n新目标\n");
    expect(diff.summary).toEqual({ added: 0, removed: 0, changed: 1 });
    const replaced = diff.lines.filter((line) => line.kind !== "same");
    expect(replaced.map((line) => [line.kind, line.text])).toEqual([
      ["removed", "旧目标"],
      ["added", "新目标"],
    ]);
  });

  it("counts 1:1 replacement as changed and longer runs as pure add or remove", () => {
    // LCS 对齐语义：b→B 是一删一增相邻（changed=1）；B1/B2 插在 a 与 b 之间、
    // b 被对齐为 same，因此是纯新增两条（changed=0）；对称地删除侧同理。
    const oneToOne = computeRevisionDiff("a\nb\nc\n", "a\nB\nc\n");
    expect(oneToOne.summary).toEqual({ added: 0, removed: 0, changed: 1 });

    const oneToThree = computeRevisionDiff("a\nb\n", "a\nB1\nB2\nb\n");
    expect(oneToThree.summary).toEqual({ added: 2, removed: 0, changed: 0 });

    const threeToOne = computeRevisionDiff("a\nb1\nb2\nb3\n", "a\nb3\n");
    expect(threeToOne.summary).toEqual({ added: 0, removed: 2, changed: 0 });
  });

  it("counts pure insertion and pure deletion", () => {
    expect(computeRevisionDiff("a\n", "a\nx\ny\n").summary).toEqual({
      added: 2,
      removed: 0,
      changed: 0,
    });
    expect(computeRevisionDiff("a\nb\nc\n", "a\n").summary).toEqual({
      added: 0,
      removed: 2,
      changed: 0,
    });
  });

  it("reports a line swap as one removal plus one addition, not a changed pair", () => {
    // LCS 对齐后 swap 的删与增被中间的 same 行隔开，成为两个独立区段——
    // 行级 diff 的诚实输出就是 +1/-1，changed 只留给同区段内的成对替换。
    const diff = computeRevisionDiff("a\nb\n", "b\na\n");
    expect(diff.summary).toEqual({ added: 1, removed: 1, changed: 0 });
  });

  it("handles empty old or new markdown", () => {
    const fromEmpty = computeRevisionDiff("", "x\n");
    expect(fromEmpty.summary).toEqual({ added: 1, removed: 0, changed: 0 });
    expect(fromEmpty.lines).toEqual([
      { kind: "added", text: "x", oldLineNo: null, newLineNo: 1 },
    ]);

    const toEmpty = computeRevisionDiff("x\n", "");
    expect(toEmpty.summary).toEqual({ added: 0, removed: 1, changed: 0 });
    expect(toEmpty.lines).toEqual([
      { kind: "removed", text: "x", oldLineNo: 1, newLineNo: null },
    ]);
  });

  it("drops a trailing newline so it does not create a phantom line", () => {
    const diff = computeRevisionDiff("a\n", "a");
    expect(diff.summary).toEqual({ added: 0, removed: 0, changed: 0 });
  });

  it("degrades to a whole-file replacement beyond the line cap", () => {
    const oldMarkdown = Array.from({ length: REVISION_DIFF_MAX_LINES + 1 }, (_, i) => `old ${i}`).join("\n");
    const newMarkdown = Array.from({ length: 3 }, (_, i) => `new ${i}`).join("\n");
    const diff = computeRevisionDiff(oldMarkdown, newMarkdown);
    expect(diff.truncated).toBe(true);
    expect(diff.summary).toEqual({
      added: 3,
      removed: REVISION_DIFF_MAX_LINES + 1,
      changed: 0,
    });
    expect(diff.lines.filter((line) => line.kind === "same")).toHaveLength(0);
  });
});

describe("toUnifiedHunks", () => {
  const CONTEXT = 3;

  it("returns no hunks when nothing changed", () => {
    const diff = computeRevisionDiff("a\nb\n", "a\nb\n");
    expect(toUnifiedHunks(diff.lines)).toEqual([]);
  });

  it("wraps a mid-file edit with context lines in one hunk", () => {
    const oldLines = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    // Array.prototype.with 需要 lib ES2023，项目 lib 为 ES2022——用等价副本改写。
    const newLines = [...oldLines];
    newLines[3] = "D";
    const diff = computeRevisionDiff(`${oldLines.join("\n")}\n`, `${newLines.join("\n")}\n`);
    const hunks = toUnifiedHunks(diff.lines, CONTEXT);
    expect(hunks).toHaveLength(1);
    // 上下文 3 + 一删一增 = 8 行切片，但两侧各有 7 行带行号（removed 无 newNo、added 无 oldNo）。
    expect(hunks[0].header).toBe("@@ -1,7 +1,7 @@");
    expect(hunks[0].lines.map((line) => line.kind)).toEqual([
      "same", "same", "same", "removed", "added", "same", "same", "same",
    ]);
  });

  it("emits two hunks for changes further apart than twice the context", () => {
    const oldLines = Array.from({ length: 20 }, (_, i) => `line ${i}`);
    const newLines = [...oldLines];
    newLines[1] = "one!";
    newLines[15] = "fifteen!";
    const diff = computeRevisionDiff(`${oldLines.join("\n")}\n`, `${newLines.join("\n")}\n`);
    const hunks = toUnifiedHunks(diff.lines, CONTEXT);
    expect(hunks).toHaveLength(2);
    expect(hunks[0].lines.some((line) => line.text === "fifteen!")).toBe(false);
    expect(hunks[1].lines.some((line) => line.text === "one!")).toBe(false);
  });

  it("uses the git-style zero header when the old side is empty", () => {
    const diff = computeRevisionDiff("", "x\ny\n");
    const hunks = toUnifiedHunks(diff.lines, CONTEXT);
    expect(hunks).toHaveLength(1);
    expect(hunks[0].header).toBe("@@ -0,0 +1,2 @@");
    expect(hunks[0].lines.every((line) => line.kind === "added")).toBe(true);
  });
});
