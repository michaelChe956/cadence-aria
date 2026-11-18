import { useEffect, useRef, useState, type FormEvent } from "react";
import { ApiRequestError } from "../../api/client";
import type {
  CodebaseSummaryDto,
  LogicalCodebaseMemberDto,
} from "../../api/types";
import { errorMessage } from "./IssueLifecycleWorkbenchParts";
import { DialogShell } from "../shared/DialogShell";

export type CreateLifecycleIssuePayload = {
  title: string;
  description: string | null;
  repository_id: string;
  /// v1.3：逻辑 issue 归属（必填）。单仓路径已下线，issue 必须落在某个 LC 上。
  logical_codebase_id: string;
  /// REQ-PIB-01：基准分支。字段保留（§0.6：独立 capability，语义不变）；
  /// 多仓路径无差异基线（Non-Goal），此处恒传 null。
  base_branch: string | null;
  /// REQ-MRE-01：LC 成员勾选集（成员列表序）。列表序首个勾选成员承载
  /// primary repository_id（勾 1 = 旧单选等价）。
  focus_repository_ids: string[];
};

type PrimaryMemberOption = {
  logical_repository_id: string;
  physical_repository_id: string | null;
  alias: string;
  /// REQ-MRE-01：服务端下发的结构化无源码标注（旧响应缺省 = 无提示）。
  /// 仅作提示，不影响复选/提交/禁用。
  index_alert: { code: string; message_zh: string } | null;
};

export function CreateLifecycleIssueDialog({
  codebases,
  listMembers,
  onCreate,
  onClose,
}: {
  /// R8：逻辑代码库列表（issue 只能挂在逻辑代码库上）。
  codebases: CodebaseSummaryDto[];
  /// R8：选中逻辑代码库时拉取 active 成员供勾选。
  listMembers: (
    logicalCodebaseId: string,
  ) => Promise<LogicalCodebaseMemberDto[]>;
  onCreate: (payload: CreateLifecycleIssuePayload) => Promise<void> | void;
  onClose: () => void;
}) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  // 代码库选择值："" | `logical_codebase_id`。
  const [codebaseValue, setCodebaseValue] = useState("");
  const [checkedLogicalIds, setCheckedLogicalIds] = useState<string[]>([]);
  const [members, setMembers] = useState<PrimaryMemberOption[]>([]);
  const [membersLoading, setMembersLoading] = useState(false);
  const [membersError, setMembersError] = useState<string | null>(null);
  const [repositoryError, setRepositoryError] = useState<string | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);

  const selectedLogicalCodebaseId = codebaseValue || null;

  useEffect(() => {
    if (!selectedLogicalCodebaseId) {
      setMembers([]);
      setMembersError(null);
      return;
    }
    let disposed = false;
    // 切 LC 生效的那一刻即清空上一 LC 的成员与标注（否则加载窗口内仍显示旧列表）。
    setMembers([]);
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
                // 标注随成员列表响应一起到达（不另发聚合索引请求）。
                index_alert: member.index_alert ?? null,
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

    if (!selectedLogicalCodebaseId) {
      setRepositoryError("请选择逻辑代码库");
      setSubmitError(null);
      return;
    }

    // REQ-MRE-01：成员复选——勾选集（成员列表序）即 focus_repository_ids；
    // 列表序首个勾选成员承载 primary repository_id（勾 1 = 旧单选等价）。
    const checked = members.filter((member) =>
      checkedLogicalIds.includes(member.logical_repository_id),
    );
    if (checked.length === 0) {
      setRepositoryError("请选择成员");
      setSubmitError(null);
      return;
    }

    submittingRef.current = true;
    setSubmitting(true);
    setRepositoryError(null);
    setSubmitError(null);
    try {
      await onCreate({
        title: title.trim(),
        description: description.trim() ? description.trim() : null,
        repository_id: checked[0].physical_repository_id ?? "",
        logical_codebase_id: selectedLogicalCodebaseId,
        // §0.6：base_branch 字段保留但多仓路径恒传 null（差异基线为 Non-Goal）。
        base_branch: null,
        focus_repository_ids: checked.map((member) => member.logical_repository_id),
      });
    } catch (reason) {
      setSubmitError(reason instanceof Error ? reason.message : "创建 Issue 失败");
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <DialogShell label="新建 Issue" onClose={onClose}>
      <form onSubmit={handleSubmit}>
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
                setCheckedLogicalIds([]);
                // 切 LC 生效即刻清空上一 LC 的成员与标注（不等到新响应到达）。
                setMembers([]);
                resetSelectionErrors();
              }}
              className="mt-1 block w-full rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
            >
              <option value="">请选择</option>
              {codebases
                .filter((codebase) => codebase.kind === "logical")
                .map((codebase) => (
                  <option
                    key={codebase.id}
                    value={codebase.logical_codebase_id ?? codebase.id}
                  >
                    {codebase.name} · 逻辑
                  </option>
                ))}
            </select>
          </label>
          {selectedLogicalCodebaseId ? (
            <fieldset className="mt-1 rounded-md border border-[var(--aria-line)] px-3 py-2">
              <legend className="px-1 text-sm font-semibold text-[var(--aria-ink)]">成员</legend>
              <span className="block text-xs font-normal text-[var(--aria-ink-muted)]">
                勾选参与本 Issue 的成员（勾选集=授权上界）；列表序首个勾选成员为 Primary。
              </span>
              <div className="mt-2 space-y-1">
                {membersLoading ? (
                  <span className="block text-sm font-normal text-[var(--aria-ink-muted)]">
                    加载成员中
                  </span>
                ) : null}
                {members.map((member) => {
                  const label = member.physical_repository_id
                    ? `${member.alias} · ${member.physical_repository_id}`
                    : `${member.alias} · 缺少物理仓库映射`;
                  return (
                    <div
                      key={member.logical_repository_id}
                      className="flex items-center gap-2"
                    >
                      <label className="flex items-center gap-2 text-sm font-normal text-[var(--aria-ink)]">
                        <input
                          type="checkbox"
                          checked={checkedLogicalIds.includes(member.logical_repository_id)}
                          disabled={!member.physical_repository_id}
                          onChange={() => {
                            setCheckedLogicalIds((previous) =>
                              previous.includes(member.logical_repository_id)
                                ? previous.filter((id) => id !== member.logical_repository_id)
                                : [...previous, member.logical_repository_id],
                            );
                            resetSelectionErrors();
                          }}
                        />
                        {label}
                      </label>
                      {/* REQ-MRE-01：服务端下发的无源码标注（正文只取 message_zh）；
                          仅提示，不影响复选/提交/禁用。 */}
                      {member.index_alert ? (
                        <span
                          data-testid={`member-index-note-${member.logical_repository_id}`}
                          className="text-xs font-normal text-[var(--aria-ink-muted)]"
                        >
                          {member.index_alert.message_zh}
                        </span>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            </fieldset>
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
    </DialogShell>
  );
}
