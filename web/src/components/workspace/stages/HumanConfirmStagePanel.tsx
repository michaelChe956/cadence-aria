import { Check, FileDiff, MessageSquareWarning, X } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import { useState } from "react";

interface ReviewerSummary {
  verdict: string;
  points: string[];
}

interface ArtifactVersionLite {
  version: number;
  markdown: string;
}

interface StructuredFeedbackPayload {
  feedback_types: string[];
  description: string;
  target_artifact_version: number;
}

interface HumanConfirmStagePanelProps {
  artifactVersion: ArtifactVersionLite;
  reviewerSummary: ReviewerSummary;
  prevVersion?: ArtifactVersionLite | null;
  onConfirm: () => void;
  onRequestChange: (feedback: StructuredFeedbackPayload) => void;
  onTerminate: () => void;
}

const FEEDBACK_TYPES = ["内容缺失", "表述不清", "与需求不符", "其他"];

export function HumanConfirmStagePanel({
  artifactVersion,
  reviewerSummary,
  prevVersion,
  onConfirm,
  onRequestChange,
  onTerminate,
}: HumanConfirmStagePanelProps) {
  const [showFeedback, setShowFeedback] = useState(false);
  const [feedbackTypes, setFeedbackTypes] = useState<string[]>([]);
  const [description, setDescription] = useState("");

  return (
    <section
      data-testid="human-confirm-panel"
      className="space-y-4 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4"
    >
      <div>
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">待人工确认</h2>
        <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
          Verdict: {reviewerSummary.verdict}
        </p>
      </div>

      <section className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
        <div className="flex items-center gap-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
          <MessageSquareWarning className="h-4 w-4 text-[var(--aria-primary)]" />
          审核摘要
        </div>
        {reviewerSummary.points.length > 0 ? (
          <ul className="mt-2 space-y-1 text-sm text-[var(--aria-ink)]">
            {reviewerSummary.points.map((point) => (
              <li key={point}>{point}</li>
            ))}
          </ul>
        ) : (
          <p className="mt-2 text-sm text-[var(--aria-ink-muted)]">无审核摘要</p>
        )}
      </section>

      {prevVersion ? (
        <section className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
          <div className="flex items-center gap-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
            <FileDiff className="h-4 w-4 text-[var(--aria-primary)]" />
            与上一版本对比
          </div>
          <div className="mt-2 text-sm font-semibold text-[var(--aria-ink)]">
            v{prevVersion.version} → v{artifactVersion.version}
          </div>
          <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
            {lineDiff(prevVersion.markdown, artifactVersion.markdown)}
          </div>
        </section>
      ) : null}

      <section className="rounded-md border border-[var(--aria-line)] bg-white p-3">
        <div className="mb-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
          Artifact 预览 v{artifactVersion.version}
        </div>
        <pre className="max-h-48 overflow-auto whitespace-pre-wrap font-mono text-xs text-[var(--aria-ink)]">
          {artifactVersion.markdown}
        </pre>
      </section>

      {!showFeedback ? (
        <div className="flex flex-wrap justify-end gap-2">
          <button
            type="button"
            onClick={onConfirm}
            className="inline-flex h-9 items-center gap-2 rounded-md border border-emerald-600 bg-emerald-600 px-3 text-sm font-semibold text-white"
          >
            <Check className="h-4 w-4" />
            确认
          </button>
          <button
            type="button"
            onClick={() => setShowFeedback(true)}
            className="inline-flex h-9 items-center gap-2 rounded-md border border-amber-200 bg-amber-50 px-3 text-sm font-semibold text-amber-800"
          >
            <MessageSquareWarning className="h-4 w-4" />
            要求修改
          </button>
          <button
            type="button"
            onClick={onTerminate}
            className="inline-flex h-9 items-center gap-2 rounded-md border border-red-200 bg-red-50 px-3 text-sm font-semibold text-red-700"
          >
            <X className="h-4 w-4" />
            终止
          </button>
        </div>
      ) : (
        <section className="space-y-3 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
          <div className="flex flex-wrap gap-2">
            {FEEDBACK_TYPES.map((type) => (
              <label
                key={type}
                className="inline-flex h-8 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-2 text-sm"
              >
                <input
                  type="checkbox"
                  checked={feedbackTypes.includes(type)}
                  onChange={(event) => toggleFeedbackType(type, event.target.checked, setFeedbackTypes)}
                />
                {type}
              </label>
            ))}
          </div>

          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            具体描述
            <textarea
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              rows={3}
              className="mt-2 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal"
            />
          </label>

          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setShowFeedback(false)}
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] bg-white px-3 text-sm font-semibold text-[var(--aria-ink)]"
            >
              取消
            </button>
            <button
              type="button"
              onClick={() =>
                onRequestChange({
                  feedback_types: feedbackTypes,
                  description,
                  target_artifact_version: artifactVersion.version,
                })
              }
              className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white"
            >
              提交
            </button>
          </div>
        </section>
      )}
    </section>
  );
}

function lineDiff(prev: string, current: string) {
  const prevLines = prev.split("\n");
  const currentLines = current.split("\n");
  const added = currentLines.filter((line) => !prevLines.includes(line)).length;
  const removed = prevLines.filter((line) => !currentLines.includes(line)).length;
  return `新增 ${added} 行 · 删除 ${removed} 行`;
}

function toggleFeedbackType(
  type: string,
  checked: boolean,
  setFeedbackTypes: Dispatch<SetStateAction<string[]>>,
) {
  setFeedbackTypes((current) =>
    checked ? [...current, type] : current.filter((candidate) => candidate !== type),
  );
}
