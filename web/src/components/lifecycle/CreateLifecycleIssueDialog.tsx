import { useEffect, useRef, useState, type FormEvent } from "react";
import { ApiRequestError } from "../../api/client";
import type {
  CodebaseSummaryDto,
  LogicalCodebaseMemberDto,
  Repository,
} from "../../api/types";
import { errorMessage } from "./IssueLifecycleWorkbenchParts";

export type CreateLifecycleIssuePayload = {
  title: string;
  description: string | null;
  repository_id: string;
  /// v1.3：逻辑 issue 归属；单仓为 null。
  logical_codebase_id: string | null;
};

type PrimaryMemberOption = {
  logical_repository_id: string;
  physical_repository_id: string | null;
  alias: string;
};

export function CreateLifecycleIssueDialog({
  repositories,
  codebases,
  listMembers,
  onCreate,
  onClose,
}: {
  repositories: Repository[];
  /// R8：混合列表（单仓 + 逻辑代码库）。
  codebases: CodebaseSummaryDto[];
  /// R8：选中逻辑代码库时拉取 active 成员供 primary 选择。
  listMembers: (
    logicalCodebaseId: string,
  ) => Promise<LogicalCodebaseMemberDto[]>;
  onCreate: (payload: CreateLifecycleIssuePayload) => Promise<void> | void;
  onClose: () => void;
}) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  // 代码库选择值："" | `repo:{repository_id}` | `lc:{logical_codebase_id}`。
  const [codebaseValue, setCodebaseValue] = useState("");
  const [primaryRepositoryId, setPrimaryRepositoryId] = useState("");
  const [members, setMembers] = useState<PrimaryMemberOption[]>([]);
  const [membersLoading, setMembersLoading] = useState(false);
  const [membersError, setMembersError] = useState<string | null>(null);
  const [repositoryError, setRepositoryError] = useState<string | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);

  const selectedLogicalCodebaseId = codebaseValue.startsWith("lc:")
    ? codebaseValue.slice("lc:".length)
    : null;

  useEffect(() => {
    if (!selectedLogicalCodebaseId) {
      setMembers([]);
      setMembersError(null);
      return;
    }
    let disposed = false;
    setMembersLoading(true);
    setMembersError(null);
    listMembers(selectedLogicalCodebaseId)
      .then((items) => {
        if (!disposed) {
          setMembers(
            (items ?? [])
              .filter((member) => member.status === "active")
              .map((member) => ({
                logical_repository_id: member.logical_repository_id,
                physical_repository_id:
                  typeof member.physical_repository_id === "string" &&
                  member.physical_repository_id.length > 0
                    ? member.physical_repository_id
                    : null,
                alias: member.alias,
              })),
          );
        }
      })
      .catch((reason: unknown) => {
        if (!disposed) {
          // M3：成员拉取失败不静默吞，展示错误信息（复用 ApiRequestError.message 模式）。
          setMembers([]);
          setMembersError(
            reason instanceof ApiRequestError
              ? reason.message
              : errorMessage(reason, "加载逻辑代码库成员失败"),
          );
        }
      })
      .finally(() => {
        if (!disposed) {
          setMembersLoading(false);
        }
      });
    return () => {
      disposed = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedLogicalCodebaseId]);

  function resetSelectionErrors() {
    setRepositoryError(null);
    setSubmitError(null);
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submittingRef.current) {
      return;
    }

    if (!codebaseValue) {
      setRepositoryError("请选择代码库");
      setSubmitError(null);
      return;
    }

    let repositoryId = "";
    let logicalCodebaseId: string | null = null;
    if (selectedLogicalCodebaseId) {
      if (!primaryRepositoryId) {
        setRepositoryError("请选择 Primary 成员");
        setSubmitError(null);
        return;
      }
      logicalCodebaseId = selectedLogicalCodebaseId;
      repositoryId = primaryRepositoryId;
    } else {
      repositoryId = codebaseValue.slice("repo:".length);
    }

    submittingRef.current = true;
    setSubmitting(true);
    setRepositoryError(null);
    setSubmitError(null);
    try {
      await onCreate({
        title: title.trim(),
        description: description.trim() ? description.trim() : null,
        repository_id: repositoryId,
        logical_codebase_id: logicalCodebaseId,
      });
    } catch (reason) {
      setSubmitError(reason instanceof Error ? reason.message : "创建 Issue 失败");
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/35 p-4">
      <form
        role="dialog"
        aria-label="新建 Issue"
        aria-modal="true"
        onSubmit={handleSubmit}
        className="w-full max-w-lg rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-base font-semibold text-[var(--aria-ink)]">新建 Issue</h2>
          <button
            type="button"
            onClick={onClose}
            className="rounded-md border border-[var(--aria-line)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]"
          >
            关闭
          </button>
        </div>
        <div className="space-y-3">
          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            Issue 标题
            <input
              value={title}
              onChange={(event) => {
                setTitle(event.target.value);
                setSubmitError(null);
              }}
              className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
            />
          </label>
          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            Issue 描述
            <textarea
              value={description}
              onChange={(event) => {
                setDescription(event.target.value);
                setSubmitError(null);
              }}
              className="mt-1 block min-h-24 w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
            />
          </label>
          <label className="block text-sm font-semibold text-[var(--aria-ink)]">
            代码库
            <select
              value={codebaseValue}
              aria-invalid={repositoryError ? "true" : undefined}
              onChange={(event) => {
                setCodebaseValue(event.target.value);
                setPrimaryRepositoryId("");
                resetSelectionErrors();
              }}
              className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
            >
              <option value="">请选择</option>
              {repositories.map((repository) => (
                <option
                  key={repository.repository_id}
                  value={`repo:${repository.repository_id}`}
                >
                  {repository.name} · 单仓
                </option>
              ))}
              {codebases
                .filter((codebase) => codebase.kind === "logical")
                .map((codebase) => (
                  <option
                    key={codebase.id}
                    value={`lc:${codebase.logical_codebase_id ?? codebase.id}`}
                  >
                    {codebase.name} · 逻辑
                  </option>
                ))}
            </select>
          </label>
          {selectedLogicalCodebaseId ? (
            <label className="block text-sm font-semibold text-[var(--aria-ink)]">
              Primary 成员
              <select
                value={primaryRepositoryId}
                disabled={membersLoading}
                onChange={(event) => {
                  setPrimaryRepositoryId(event.target.value);
                  resetSelectionErrors();
                }}
                className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)] disabled:opacity-60"
              >
                <option value="">
                  {membersLoading ? "加载成员中" : "请选择"}
                </option>
                {members.map((member) => (
                  <option
                    key={member.logical_repository_id}
                    value={member.physical_repository_id ?? ""}
                    disabled={!member.physical_repository_id}
                  >
                    {member.alias}
                    {member.physical_repository_id
                      ? ` · ${member.physical_repository_id}`
                      : " · 缺少物理仓库映射"}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
          {membersError ? (
            <p role="alert" className="text-sm font-semibold text-[var(--aria-danger)]">
              成员加载失败：{membersError}
            </p>
          ) : null}
          {repositoryError ? (
            <p className="text-sm font-semibold text-[var(--aria-danger)]">{repositoryError}</p>
          ) : null}
          {submitError ? (
            <p role="alert" className="text-sm font-semibold text-[var(--aria-danger)]">
              {submitError}
            </p>
          ) : null}
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink-muted)]"
          >
            取消
          </button>
          <button
            type="submit"
            disabled={submitting}
            className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-2 text-sm font-semibold text-white disabled:opacity-60"
          >
            创建 Issue
          </button>
        </div>
      </form>
    </div>
  );
}
