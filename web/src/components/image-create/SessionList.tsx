import { ImagePlus, MessageSquare, Plus, Trash2, X } from "lucide-react";
import { useState, type FormEvent } from "react";
import {
  IMAGE_CREATE_PROVIDER_OPTIONS,
  type ImageCreatePreset,
  type ImageCreateProvider,
  type ImageCreateTemplateChoice,
  type SessionSummary,
} from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";
import { CustomSelect } from "./CustomSelect";

type TemplateSelection = ImageCreatePreset | "custom";

const TEMPLATE_LABELS: Record<ImageCreatePreset, string> = {
  ppt_business_illustration: "PPT 商务配图",
  business_flow_diagram: "业务流程图",
  web_page_ui: "Web 页面 UI 图",
};

const TEMPLATE_SELECTION_OPTIONS = [
  ...Object.values(TEMPLATE_LABELS),
  "自定义引导词",
] as const;

function templateSelectionLabel(selection: TemplateSelection): string {
  return selection === "custom" ? "自定义引导词" : TEMPLATE_LABELS[selection];
}

function templateSelectionFromLabel(label: string): TemplateSelection {
  if (label === "自定义引导词") {
    return "custom";
  }
  const selection = Object.entries(TEMPLATE_LABELS).find(
    ([, templateName]) => templateName === label,
  )?.[0];
  return (selection ?? "ppt_business_illustration") as ImageCreatePreset;
}

const fieldClassName =
  "mt-1.5 block w-full rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3.5 py-2.5 text-sm font-medium text-[var(--aria-ink)] shadow-[inset_0_1px_2px_rgba(15,23,42,0.04)] transition-all duration-200 hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel)] focus-visible:border-[var(--aria-primary)] focus-visible:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2";

export function SessionList({
  onClose,
  onSessionSelect,
}: {
  onClose?: () => void;
  onSessionSelect?: () => void;
} = {}) {
  const sessions = useImageCreateStore((state) => state.sessions);
  const currentSessionId = useImageCreateStore(
    (state) => state.currentSession?.session.id ?? null,
  );
  const openSession = useImageCreateStore((state) => state.openSession);
  const createSession = useImageCreateStore((state) => state.createSession);
  const deleteSession = useImageCreateStore((state) => state.deleteSession);
  const [showCreate, setShowCreate] = useState(false);
  const [template, setTemplate] = useState<TemplateSelection>(
    "ppt_business_illustration",
  );
  const [provider, setProvider] = useState<ImageCreateProvider>("claude_code");
  const [customTemplate, setCustomTemplate] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submitting) {
      return;
    }
    const choice: ImageCreateTemplateChoice =
      template === "custom"
        ? { custom: customTemplate.trim() }
        : { preset: template };
    if (template === "custom" && !customTemplate.trim()) {
      setError("请输入自定义引导词");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await createSession(choice, provider);
      setShowCreate(false);
      setTemplate("ppt_business_illustration");
      setProvider("claude_code");
      setCustomTemplate("");
      onSessionSelect?.();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "创建会话失败");
    } finally {
      setSubmitting(false);
    }
  }

  async function handleDelete(sessionId: string) {
    setError(null);
    try {
      await deleteSession(sessionId);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "删除会话失败");
    }
  }

  return (
    <aside className="flex h-full min-h-0 flex-col overflow-hidden rounded-2xl border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-[0_2px_8px_rgba(15,23,42,0.04),0_10px_28px_rgba(15,23,42,0.06)] transition-all duration-200 hover:shadow-md">
      <div className="flex items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-gradient-to-b from-white to-[var(--aria-panel-muted)] p-4">
        <div className="flex items-center gap-2.5">
          <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
            <MessageSquare aria-hidden="true" className="h-4 w-4" />
          </span>
          <div>
            <h2 className="text-base font-semibold">会话</h2>
            <p className="text-xs text-[var(--aria-ink-muted)]">保存每次创作思路</p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {onClose ? (
            <button
              type="button"
              aria-label="关闭会话列表"
              onClick={onClose}
              className="inline-flex min-h-11 min-w-11 cursor-pointer items-center justify-center rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-all duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 lg:hidden"
            >
              <X aria-hidden="true" className="h-5 w-5" />
            </button>
          ) : null}
          <button
            type="button"
            aria-label="新建会话"
            title="新建会话"
            onClick={() => {
              setShowCreate(true);
              setError(null);
            }}
            className="inline-flex min-h-11 min-w-11 cursor-pointer items-center justify-center rounded-xl bg-gradient-to-b from-[var(--aria-primary)] to-[#0e7490] text-white shadow-[0_4px_12px_rgba(8,145,178,0.25)] transition-all duration-200 hover:translate-y-px hover:shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0.5"
          >
            <Plus aria-hidden="true" className="h-4 w-4" />
          </button>
        </div>
      </div>
      {error ? (
        <p role="alert" className="mx-3 mt-3 rounded-lg border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
          {error}
        </p>
      ) : null}
      <div className="min-h-0 flex-1 space-y-2.5 overflow-y-auto p-3">
        {sessions.length === 0 ? (
          <div className="flex min-h-52 flex-col items-center justify-center rounded-2xl border border-dashed border-[var(--aria-line-strong)] bg-[var(--aria-panel-muted)] px-4 py-8 text-center">
            <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
              <ImagePlus aria-hidden="true" className="h-6 w-6" />
            </span>
            <p className="mt-3 text-sm font-semibold text-[var(--aria-ink)]">暂无创作会话</p>
            <p className="mt-1 text-xs leading-5 text-[var(--aria-ink-muted)]">新建会话，选择模板开始构思图片。</p>
          </div>
        ) : (
          sessions.map((session) => (
            <SessionItem
              key={session.id}
              session={session}
              active={currentSessionId === session.id}
              onOpen={() => {
                openSession(session.id).catch(() => {});
                onSessionSelect?.();
              }}
              onDelete={() => void handleDelete(session.id)}
            />
          ))
        )}
      </div>
      {showCreate ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/40 p-4 backdrop-blur-sm">
          <form
            role="dialog"
            aria-modal="true"
            aria-label="创建图片创作会话"
            onSubmit={handleCreate}
            className="w-full max-w-md rounded-2xl border border-white/80 bg-[var(--aria-panel)] p-5 shadow-[0_24px_64px_rgba(15,23,42,0.2)]"
          >
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-3">
                <span className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--aria-primary-soft)] text-[var(--aria-primary)] shadow-sm">
                  <ImagePlus aria-hidden="true" className="h-5 w-5" />
                </span>
                <div>
                  <h3 className="text-base font-semibold">新建会话</h3>
                  <p className="mt-0.5 text-xs text-[var(--aria-ink-muted)]">选择一个创作起点</p>
                </div>
              </div>
              <button
                type="button"
                aria-label="关闭"
                onClick={() => setShowCreate(false)}
                className="inline-flex min-h-11 min-w-11 cursor-pointer items-center justify-center rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-all duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
              >
                <X aria-hidden="true" className="h-4 w-4" />
              </button>
            </div>
            <div className="mt-5 space-y-4 rounded-xl bg-[var(--aria-panel-muted)] p-4">
              <CustomSelect
                label="模板"
                value={templateSelectionLabel(template)}
                options={TEMPLATE_SELECTION_OPTIONS}
                onChange={(value) => {
                  setTemplate(templateSelectionFromLabel(value));
                  setError(null);
                }}
              />
              {template === "custom" ? (
                <label className="block text-sm font-semibold">
                  自定义引导词
                  <textarea
                    aria-label="自定义引导词"
                    value={customTemplate}
                    onChange={(event) => {
                      setCustomTemplate(event.target.value);
                      setError(null);
                    }}
                    rows={3}
                    placeholder="描述这个会话的创作方向"
                    className={`${fieldClassName} min-h-11 resize-y text-base placeholder:text-[var(--aria-ink-muted)] sm:text-sm`}
                  />
                </label>
              ) : null}
              <CustomSelect
                label="Provider"
                value={provider}
                options={IMAGE_CREATE_PROVIDER_OPTIONS}
                onChange={(value) =>
                  setProvider(value as ImageCreateProvider)
                }
              />
              {error ? (
                <p role="alert" className="rounded-lg border border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
                  {error}
                </p>
              ) : null}
            </div>
            <div className="mt-5 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setShowCreate(false)}
                className="min-h-11 cursor-pointer rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2.5 text-sm font-semibold text-[var(--aria-ink-muted)] shadow-sm transition-all duration-200 hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
              >
                取消
              </button>
              <button
                type="submit"
                disabled={submitting}
                className="min-h-11 cursor-pointer rounded-xl bg-gradient-to-b from-[var(--aria-primary)] to-[#0e7490] px-4 py-2.5 text-sm font-semibold text-white shadow-[0_4px_12px_rgba(8,145,178,0.24)] transition-all duration-200 hover:translate-y-px hover:shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 active:translate-y-0.5 disabled:cursor-not-allowed disabled:opacity-60"
              >
                {submitting ? "创建中…" : "创建"}
              </button>
            </div>
          </form>
        </div>
      ) : null}
    </aside>
  );
}

function SessionItem({
  session,
  active,
  onOpen,
  onDelete,
}: {
  session: SessionSummary;
  active: boolean;
  onOpen: () => void;
  onDelete: () => void;
}) {
  return (
    <div
      className={`group rounded-xl border p-2.5 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-sm ${
        active
          ? "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] shadow-[inset_0_1px_0_rgba(255,255,255,0.8),0_4px_12px_rgba(8,145,178,0.1)]"
          : "border-[var(--aria-line)] bg-[var(--aria-panel)] hover:border-[var(--aria-line-strong)] hover:bg-[var(--aria-panel-muted)]"
      }`}
    >
      <button
        type="button"
        onClick={onOpen}
        className="min-h-11 w-full cursor-pointer rounded-lg px-1.5 py-1 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
      >
        <span className="block truncate text-sm font-semibold">
          {templateLabel(session.template)}
        </span>
        <span className="mt-1.5 block text-xs text-[var(--aria-ink-muted)]">
          {session.provider_name} · {session.status}
        </span>
        <span className="mt-1 block truncate font-mono text-[10px] text-[var(--aria-ink-muted)]">
          {session.id}
        </span>
      </button>
      <div className="mt-1 flex justify-end">
        <button
          type="button"
          aria-label={`删除会话 ${session.id}`}
          onClick={onDelete}
          disabled={session.status === "deleting"}
          className="inline-flex min-h-11 cursor-pointer items-center gap-1 rounded-lg px-3 py-2 text-xs font-semibold text-[var(--aria-danger)] transition-all duration-200 hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)] focus-visible:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50"
        >
          <Trash2 aria-hidden="true" className="h-3 w-3" />
          删除
        </button>
      </div>
    </div>
  );
}

function templateLabel(template: ImageCreateTemplateChoice) {
  if (template.preset) {
    return TEMPLATE_LABELS[template.preset];
  }
  return template.custom?.trim() || "自定义引导词";
}
