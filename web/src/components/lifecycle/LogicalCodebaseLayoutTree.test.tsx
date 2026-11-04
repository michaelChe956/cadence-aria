import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import wizardSource from "./LogicalCodebaseRegistrationWizard.tsx?raw";
import wizardTestSource from "./LogicalCodebaseRegistrationWizard.test.tsx?raw";
import { LogicalCodebaseLayoutTree } from "./LogicalCodebaseLayoutTree";
import treeSource from "./LogicalCodebaseLayoutTree.tsx?raw";

/** 片5 生产文件的源码文本（Vite `?raw` 导入，避免依赖 node 内建模块与 @types/node）。 */
const PRODUCTION_SOURCES: Record<string, string> = {
  "LogicalCodebaseLayoutTree.tsx": treeSource,
  "LogicalCodebaseRegistrationWizard.tsx": wizardSource,
};

/** 片5 含测试的全部源码文本（用于"不得引用已取消检索面"的全量锁）。 */
const ALL_SLICE_SOURCES: Record<string, string> = {
  ...PRODUCTION_SOURCES,
  "LogicalCodebaseRegistrationWizard.test.tsx": wizardTestSource,
};

describe("LogicalCodebaseLayoutTree", () => {
  it("tree_shows_root_and_direct_children", () => {
    render(<LogicalCodebaseLayoutTree />);

    // LC 根是普通空目录（不是 git 仓）；成员仓是其直接子目录。
    expect(screen.getByTestId("lc-layout-tree-root")).toHaveTextContent(
      "my-lc-root/",
    );
    expect(screen.getByTestId("lc-layout-tree-root")).toHaveTextContent(
      "不是 git 仓",
    );
    expect(
      screen.getByTestId("lc-layout-tree-member-backend"),
    ).toHaveTextContent("backend/");
    expect(
      screen.getByTestId("lc-layout-tree-member-frontend"),
    ).toHaveTextContent("frontend/");
    // 成员仓必须直接位于根下（「直接子目录」而非任意深度）。
    expect(
      screen.getByTestId("lc-layout-tree-member-backend"),
    ).toHaveTextContent("直接子目录");
    expect(
      screen.getByTestId("lc-layout-tree-member-frontend"),
    ).toHaveTextContent("直接子目录");
  });

  it("tree_explains_worktrees_are_not_indexable", () => {
    render(<LogicalCodebaseLayoutTree />);

    expect(screen.getByTestId("lc-layout-tree-note")).toHaveTextContent(
      "只在 .worktrees/ 下的代码不可被索引",
    );
  });

  it("slice5_files_contain_no_layout_copy_constants", () => {
    // ① 生产文件不得出现布局文案表 / 第二份中文模板（REQ-LAYOUT-04 / REQ-LAYOUT-05）。
    // 字段名字面量按片段拼接，避免本断言自己的源码被误判。
    // 扫描范围只含生产文件：测试夹具必须按线上字段名构造 mock 响应（那是后端
    // 形状的回放，不是第二份文案来源）；测试文件另由 ②' 锁模块面。
    const ZH_KEY_SUFFIX = "_zh";
    const copyTableEntry = new RegExp(
      `\\b(?:problem|reason|action)${ZH_KEY_SUFFIX}\\s*:\\s*["'\`]`,
    );
    const copyTableName = new RegExp(
      ["failures", "zh"].join("_") +
        "|" +
        ["LAYOUT", "FAILURE", "COPY"].join("_"),
    );
    for (const [name, source] of Object.entries(PRODUCTION_SOURCES)) {
      expect(copyTableEntry.test(source), name).toBe(false);
      expect(copyTableName.test(source), name).toBe(false);
    }

    // ② 生产文件不得引用已取消的独立检索面 / 动作组件。
    // 组件名 / 模块名按片段拼接，避免本断言自己的源码被误判（下同）。
    const GUIDANCE_MODULE = ["layout", "guidance"].join("[_-]?");
    const ACTION_COMPONENT = ["Layout", "Failure", "Action"].join("");
    const cancelledModules = new RegExp(
      `${GUIDANCE_MODULE}|${ACTION_COMPONENT}`,
      "i",
    );
    for (const [name, source] of Object.entries(PRODUCTION_SOURCES)) {
      expect(cancelledModules.test(source), name).toBe(false);
    }

    // ②' 片 5 的全部文件（含测试）都不得引用已取消的检索面 / 动作组件。
    // 磁盘"未建"由报告里的 grep 自证（layout_guidance.rs / 布局端点 / LayoutFailureAction）。
    for (const [name, source] of Object.entries(ALL_SLICE_SOURCES)) {
      expect(cancelledModules.test(source), name).toBe(false);
    }
  });
});
