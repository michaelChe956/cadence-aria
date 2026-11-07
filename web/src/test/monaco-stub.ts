/**
 * 测试环境下的 `monaco-editor` 替身（stub）。
 *
 * 背景：jsdom 不实现 `document.queryCommandSupported` 等浏览器 API，
 * 而 `monaco-editor` 主包在**模块求值期**就会调用它们
 * （`esm/vs/editor/contrib/clipboard/browser/clipboard.js`）
 * ⇒ 只要测试间接 import 到 `monaco-local-setup.ts`，整个测试文件就会在**收集阶段**抛错。
 *
 * 因此在 vitest 配置里把 `monaco-editor` 别名指向本模块
 * （见 `web/vite.config.ts` 的 `test.alias`）：
 * 测试只关心「组件是否渲染出预期结构」，不需要真实编辑器内核。
 *
 * ★本模块只在测试环境生效，生产构建仍使用真实的 `monaco-editor` 包。
 * ★覆盖面依据 `@monaco-editor/react` v4.7 的 `Editor` / `DiffEditor` 实现
 *   （其 `utils/getOrCreateModel`、`Editor.tsx`、`DiffEditor.tsx`），
 *   只实现被这些组件真实调用到的 API。
 */

/** 订阅句柄（对齐 monaco 的 `IDisposable`）。 */
export interface StubDisposable {
  dispose(): void;
}

export interface StubUri {
  scheme: string;
  path: string;
  toString(): string;
}

export interface StubRange {
  startLineNumber: number;
  startColumn: number;
  endLineNumber: number;
  endColumn: number;
}

export interface StubModel {
  readonly uri: StubUri;
  getValue(): string;
  setValue(value: string): void;
  getFullModelRange(): StubRange;
  getLanguageId(): string;
  setLanguage(language: string): void;
  isDisposed(): boolean;
  dispose(): void;
  onDidChangeContent(listener: () => void): StubDisposable;
}

export interface StubEditor {
  getModel(): StubModel | null;
  setModel(model: StubModel | null): void;
  getValue(): string;
  setValue(value: string): void;
  getOption(option: string): unknown;
  updateOptions(options: Record<string, unknown>): void;
  executeEdits(source: string, edits: unknown[]): void;
  pushUndoStop(): void;
  revealLine(line: number): void;
  layout(): void;
  focus(): void;
  saveViewState(): null;
  restoreViewState(state: unknown): void;
  onDidChangeModelContent(listener: () => void): StubDisposable;
  dispose(): void;
}

export interface StubDiffEditor {
  getOriginalEditor(): StubEditor;
  getModifiedEditor(): StubEditor;
  getModel(): { original: StubModel; modified: StubModel } | null;
  setModel(models: { original: StubModel; modified: StubModel }): void;
  updateOptions(options: Record<string, unknown>): void;
  layout(): void;
  dispose(): void;
}

function createDisposable(): StubDisposable {
  return { dispose: () => {} };
}

class StubModelImpl implements StubModel {
  readonly uri: StubUri;
  private value: string;
  private language: string;
  private disposed = false;
  private readonly listeners = new Set<() => void>();

  constructor(value: string, language: string, uri: StubUri) {
    this.value = value;
    this.language = language;
    this.uri = uri;
  }

  getValue(): string {
    return this.value;
  }

  setValue(value: string): void {
    this.value = value;
    for (const listener of Array.from(this.listeners)) listener();
  }

  getFullModelRange(): StubRange {
    return {
      startLineNumber: 1,
      startColumn: 1,
      endLineNumber: 1,
      endColumn: this.value.length + 1,
    };
  }

  getLanguageId(): string {
    return this.language;
  }

  setLanguage(language: string): void {
    this.language = language || "plaintext";
  }

  isDisposed(): boolean {
    return this.disposed;
  }

  dispose(): void {
    this.disposed = true;
    this.listeners.clear();
  }

  onDidChangeContent(listener: () => void): StubDisposable {
    this.listeners.add(listener);
    return {
      dispose: () => {
        this.listeners.delete(listener);
      },
    };
  }
}

class StubEditorImpl implements StubEditor {
  private model: StubModel | null;
  private options: Record<string, unknown>;

  constructor(options: Record<string, unknown>) {
    this.options = { ...options };
    this.model = (options.model as StubModel | undefined) ?? null;
  }

  getModel(): StubModel | null {
    return this.model;
  }

  setModel(model: StubModel | null): void {
    this.model = model;
  }

  getValue(): string {
    return this.model?.getValue() ?? "";
  }

  setValue(value: string): void {
    this.model?.setValue(value);
  }

  getOption(option: string): unknown {
    return this.options[option];
  }

  updateOptions(options: Record<string, unknown>): void {
    this.options = { ...this.options, ...options };
  }

  executeEdits(): void {
    /* 测试环境无需真实编辑 */
  }

  pushUndoStop(): void {
    /* 测试环境无需真实撤销栈 */
  }

  revealLine(): void {
    /* 测试环境无需真实滚动 */
  }

  layout(): void {
    /* 测试环境无需真实布局 */
  }

  focus(): void {
    /* 测试环境无需真实焦点 */
  }

  saveViewState(): null {
    return null;
  }

  restoreViewState(): void {
    /* 测试环境无需真实视图状态 */
  }

  onDidChangeModelContent(): StubDisposable {
    return createDisposable();
  }

  dispose(): void {
    this.model = null;
  }
}

class StubDiffEditorImpl implements StubDiffEditor {
  private readonly original: StubEditor;
  private readonly modified: StubEditor;
  private models: { original: StubModel; modified: StubModel } | null = null;

  constructor(options: Record<string, unknown>) {
    this.original = new StubEditorImpl(options);
    this.modified = new StubEditorImpl(options);
  }

  getOriginalEditor(): StubEditor {
    return this.original;
  }

  getModifiedEditor(): StubEditor {
    return this.modified;
  }

  getModel(): { original: StubModel; modified: StubModel } | null {
    return this.models;
  }

  setModel(models: { original: StubModel; modified: StubModel }): void {
    this.models = models;
    this.original.setModel(models.original);
    this.modified.setModel(models.modified);
  }

  updateOptions(options: Record<string, unknown>): void {
    this.original.updateOptions(options);
    this.modified.updateOptions(options);
  }

  layout(): void {
    /* 测试环境无需真实布局 */
  }

  dispose(): void {
    this.models = null;
  }
}

/** monaco 的 `Uri.parse`（只需满足 `@monaco-editor/react` 的 `createModelUri`）。 */
export const Uri = {
  parse(value: string): StubUri {
    const separator = value.indexOf(":");
    const scheme = separator >= 0 ? value.slice(0, separator) : "";
    const path = separator >= 0 ? value.slice(separator + 1) : value;
    return {
      scheme,
      path,
      toString: () => value,
    };
  },
};

const models = new Map<string, StubModel>();
let lastModel: StubModel | null = null;

function registerModel(model: StubModel): StubModel {
  models.set(model.uri.toString(), model);
  lastModel = model;
  return model;
}

function createModelInternal(
  value: string,
  language: string,
  uri?: StubUri,
): StubModel {
  const model = new StubModelImpl(
    value,
    language || "plaintext",
    uri ?? Uri.parse(`stub://model/${models.size + 1}`),
  );
  return registerModel(model);
}

const editor = {
  /** `EditorOption` 只需覆盖 `@monaco-editor/react` 读取的 `readOnly`。 */
  EditorOption: { readOnly: "readOnly" } as Record<string, string>,

  create(container: unknown, options: Record<string, unknown>): StubEditor {
    return new StubEditorImpl(options ?? {});
  },

  createDiffEditor(
    container: unknown,
    options: Record<string, unknown>,
  ): StubDiffEditor {
    return new StubDiffEditorImpl(options ?? {});
  },

  createModel(
    value = "",
    language = "plaintext",
    uri?: StubUri,
  ): StubModel {
    return createModelInternal(value, language, uri);
  },

  getModel(uri?: StubUri): StubModel | null {
    if (!uri) return lastModel;
    return models.get(uri.toString()) ?? null;
  },

  setModelLanguage(model: StubModel, language: string): void {
    if (!model || !language) return;
    model.setLanguage(language);
  },

  setTheme(_theme: string): void {
    /* 测试环境无需真实主题 */
  },

  defineTheme(): void {
    /* 测试环境无需真实主题 */
  },

  getModelMarkers(): unknown[] {
    return [];
  },

  onDidChangeMarkers(): StubDisposable {
    return createDisposable();
  },
};

const monaco = {
  editor,
  Uri,
};

export { editor };

export default monaco;
