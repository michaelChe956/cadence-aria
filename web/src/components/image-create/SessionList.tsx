import { useState, type FormEvent } from "react";
import {
  IMAGE_CREATE_PROVIDER_OPTIONS,
  type ImageCreatePreset,
  type ImageCreateProvider,
  type ImageCreateTemplateChoice,
  type SessionSummary,
} from "../../api/types/image-create";
import { useImageCreateStore } from "../../state/image-create-store";

type TemplateSelection = ImageCreatePreset | "custom";

const TEMPLATE_LABELS: Record<ImageCreatePreset, string> = {
  ppt_business_illustration: "PPT 商务配图",
  business_flow_diagram: "业务流程图",
};

export function SessionList() {
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
    <aside className="flex min-h-0 flex-col rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-sm">
      <div className="flex items-center justify-between gap-3 border-b border-[var(--aria-line)] p-4">
        <h2 className="text-base font-semibold">会话</h2>
        <button
          type="button"
          onClick={() => {
            setShowCreate(true);
            setError(null);
          }}
          className="rounded-md bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white"
        >
          新建会话
        </button>
      </div>
      {error ? (
        <p role="alert" className="mx-3 mt-3 rounded-md bg-red-50 px-3 py-2 text-sm text-red-700">
          {error}
        </p>
      ) : null}
      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto p-3">
        {sessions.length === 0 ? (
          <p className="rounded-lg border border-dashed border-[var(--aria-line)] px-3 py-8 text-center text-sm text-[var(--aria-ink-muted)]">
            暂无会话
          </p>
        ) : (
          sessions.map((session) => (
            <SessionItem
              key={session.id}
              session={session}
              active={currentSessionId === session.id}
              onOpen={() => {
                openSession(session.id).catch(() => {});
              }}
              onDelete={() => void handleDelete(session.id)}
            />
          ))
        )}
      </div>
      {showCreate ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4">
          <form
            role="dialog"
            aria-modal="true"
            aria-label="创建图片创作会话"
            onSubmit={handleCreate}
            className="w-full max-w-md rounded-xl border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
          >
            <div className="flex items-center justify-between gap-3">
              <h3 className="text-base font-semibold">新建会话</h3>
              <button
                type="button"
                onClick={() => setShowCreate(false)}
                className="rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold"
              >
                关闭
              </button>
            </div>
            <div className="mt-4 space-y-3">
              <label className="block text-sm font-semibold">
                模板
                <select
                  aria-label="模板"
                  value={template}
                  onChange={(event) => {
                    setTemplate(event.target.value as TemplateSelection);
                    setError(null);
                  }}
                  className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal"
                >
                  <option value="ppt_business_illustration">PPT 商务配图</option>
                  <option value="business_flow_diagram">业务流程图</option>
                  <option value="custom">自定义引导词</option>
                </select>
              </label>
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
                    className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal"
                  />
                </label>
              ) : null}
              <label className="block text-sm font-semibold">
                Provider
                <select
                  aria-label="Provider"
                  value={provider}
                  onChange={(event) =>
                    setProvider(event.target.value as ImageCreateProvider)
                  }
                  className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal"
                >
                  {IMAGE_CREATE_PROVIDER_OPTIONS.map((option) => (
                    <option key={option} value={option}>
                      {option}
                    </option>
                  ))}
                </select>
              </label>
              {error ? (
                <p role="alert" className="text-sm font-semibold text-[var(--aria-danger)]">
                  {error}
                </p>
              ) : null}
            </div>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setShowCreate(false)}
                className="rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm font-semibold"
              >
                取消
              </button>
              <button
                type="submit"
                disabled={submitting}
                className="rounded-md bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
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
      className={`rounded-lg border p-2 ${
        active
          ? "border-[var(--aria-primary)] bg-blue-50"
          : "border-[var(--aria-line)] bg-white"
      }`}
    >
      <button type="button" onClick={onOpen} className="w-full px-1 py-1 text-left">
        <span className="block truncate text-sm font-semibold">
          {templateLabel(session.template)}
        </span>
        <span className="mt-1 block text-xs text-[var(--aria-ink-muted)]">
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
          className="rounded px-2 py-1 text-xs font-semibold text-[var(--aria-danger)] hover:bg-red-50 disabled:opacity-50"
        >
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
