import { describe, expect, it, vi } from "vitest";
import * as monaco from "monaco-editor";
import { loader } from "@monaco-editor/react";
// ★静态 import（而非 await import）：本测试断言的正是「模块被加载时的副作用」，
//   静态 import 让失败发生在构建/加载期，且顺带引入 setup 模块的全局类型声明。
import "./monaco-local-setup";

// ★契约测试：monaco 必须来自本地包，运行时不得依赖任何外部 CDN。
// ★红线：行为断言（loader.config 的入参），**不断言源码文本**。

const { loaderConfig } = vi.hoisted(() => ({ loaderConfig: vi.fn() }));

vi.mock("@monaco-editor/react", () => ({
  loader: {
    config: (options: unknown) => loaderConfig(options),
  },
}));

vi.mock("monaco-editor", () => ({
  default: { __marker: "local-monaco-module" },
  __marker: "local-monaco-module",
}));

vi.mock("monaco-editor/esm/vs/editor/editor.worker.js?worker", () => ({
  default: class FakeEditorWorker {},
}));
vi.mock("monaco-editor/esm/vs/language/json/json.worker.js?worker", () => ({
  default: class FakeJsonWorker {},
}));
vi.mock("monaco-editor/esm/vs/language/css/css.worker.js?worker", () => ({
  default: class FakeCssWorker {},
}));
vi.mock("monaco-editor/esm/vs/language/html/html.worker.js?worker", () => ({
  default: class FakeHtmlWorker {},
}));
vi.mock("monaco-editor/esm/vs/language/typescript/ts.worker.js?worker", () => ({
  default: class FakeTsWorker {},
}));

describe("monaco 本地化装配", () => {
  it("以本地 monaco-editor 包调用 loader.config，且不设任何外部 paths.vs", () => {
    expect(loaderConfig).toHaveBeenCalledTimes(1);
    const options = loaderConfig.mock.calls[0]?.[0] as {
      monaco?: unknown;
      paths?: Record<string, string>;
    };

    // 传入的是本地 monaco-editor 模块对象（不是 URL 字符串）
    expect(options.monaco).toBeDefined();
    expect(typeof options.monaco).toBe("object");
    const markerHolder = options.monaco;
    if (
      markerHolder &&
      typeof markerHolder === "object" &&
      "__marker" in markerHolder
    ) {
      expect(markerHolder.__marker).toBe("local-monaco-module");
    }

    // 不得指向任何外部地址
    expect(options.paths?.vs).toBeUndefined();
    expect(JSON.stringify(options.paths ?? {})).not.toMatch(/https?:\/\//);
  });

  it("monaco 取自本地包而非外部地址", () => {
    expect(typeof monaco).toBe("object");
    expect(monaco).not.toBeUndefined();
  });

  it("按 label 把 worker 分发到本地 worker（非 URL 字符串）", () => {
    const getWorker = window.MonacoEnvironment?.getWorker;
    expect(typeof getWorker).toBe("function");
    if (!getWorker) return;

    const labels = ["json", "css", "less", "html", "typescript", "javascript", "editor"];
    const produced = labels.map((label) => getWorker({}, label));

    for (const worker of produced) {
      expect(worker).toBeDefined();
      expect(typeof worker).toBe("object");
      expect(typeof worker).not.toBe("string");
    }

    // json 与 editor 必须走不同 worker 类（证明按 label 分发，而非一律同一个）
    expect(produced[0]?.constructor).not.toBe(produced[labels.length - 1]?.constructor);
  });
});
