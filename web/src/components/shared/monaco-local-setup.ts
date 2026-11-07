import * as monaco from "monaco-editor";
import { loader } from "@monaco-editor/react";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker.js?worker";
import JsonWorker from "monaco-editor/esm/vs/language/json/json.worker.js?worker";
import CssWorker from "monaco-editor/esm/vs/language/css/css.worker.js?worker";
import HtmlWorker from "monaco-editor/esm/vs/language/html/html.worker.js?worker";
import TsWorker from "monaco-editor/esm/vs/language/typescript/ts.worker.js?worker";

declare global {
  interface Window {
    MonacoEnvironment?: {
      getWorker: (moduleId: unknown, label: string) => Worker;
    };
  }
}

/**
 * monaco 本地化装配（消除 CDN 依赖）。
 *
 * 背景：`@monaco-editor/react` 不配 `loader.config` 时，由 `monaco-editor`
 * 包自己的 loader 把 `paths.vs` 指向 `https://cdn.jsdelivr.net/...`
 * ⇒ 运行时必须能出网；无外网时 monaco 永不初始化 ⇒ Artifact 产物区白屏。
 *
 * 本模块必须被**任何使用 `Editor` / `DiffEditor` 的组件之前** import
 * （`@monaco-editor/react` 官方 `setupMonaco` 模式）：`loader.config`
 * 必须在该包初始化 loader 之前生效。
 */
window.MonacoEnvironment = {
  getWorker: (_moduleId: unknown, label: string): Worker => {
    if (label === "json") return new JsonWorker();
    if (label === "css" || label === "scss" || label === "less") return new CssWorker();
    if (label === "html" || label === "handlebars" || label === "razor") return new HtmlWorker();
    if (label === "typescript" || label === "javascript") return new TsWorker();
    return new EditorWorker();
  },
};

loader.config({ monaco });
