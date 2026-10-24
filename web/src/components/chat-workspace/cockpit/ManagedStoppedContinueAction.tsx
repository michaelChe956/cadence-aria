// 批 2 P5 补入口（REQ-WIGA-08/10）：托管 stopped 卡（精确绑定当前 enabled
// enrollment 的 plan session）走不了 takeover（服务端一律拒、收件箱隐藏接管），
// 唯一合法恢复路线是「同一 plan 对话窗口补说明＋继续」。本组件把 continue-plan
// 搬进卡片原地：
//   - 入口是否出现只镜像 P4 `GET …/automation-enrollment/control` 的事实
//     （managed＋未 terminated＋stop_key/binding 身份齐备＋allowed_actions 含
//     continue_plan）；读取失败一律 fail-closed 不出动作面。
//   - 打开时与提交前各重读一次 control：expected_binding/expected_stop_key/
//     context_required 一律取当次 GET，不复用旧值、不猜。
//   - 回执照既有 P5 语义：accepted/replayed=已出站（清空输入、lifecycle
//     invalidation、卡片状态复读）；needs_human=保留分诊面；4xx/网络失败=
//     卡内内联后端 code+message、保留输入、不自动重试。
//   - 不发任何新协议：REST 通道与对话窗口恢复面完全同源（client.ts）。
import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { RefreshCcw } from "lucide-react";
import {
  findWorkspaceSessionIssueAddress,
  getAutomationEnrollmentControl,
  postAutomationContinuePlan,
  type AutomationControlStatus,
} from "../../../api/client";
import { newCommandId } from "../../../hooks/useWorkspaceWs";
import { notifyLifecycleInvalidated } from "../../../state/lifecycle-workbench-store";
import { BTN_PRIMARY_CLASS, BTN_SECONDARY_CLASS } from "../gate-visual-tokens";

type ContinueNotice = {
  tone: "info" | "warn" | "danger";
  text: string;
};

/** 一次 control 读取的可用事实（身份齐备且允许 continue_plan 时才会构造）。 */
type ContinueFacts = {
  projectId: string;
  issueId: string;
  binding: NonNullable<AutomationControlStatus["binding"]>;
  stopKey: string;
  contextRequired: boolean;
};

/**
 * 会话 id → issue 地址 → control 只读投影。任一步失败/事实不满足返回 null
 *（调用方 fail-closed 不出动作面，不猜身份）。
 */
async function loadContinueFacts(sessionId: string): Promise<ContinueFacts | null> {
  try {
    const address = await findWorkspaceSessionIssueAddress(sessionId);
    if (address === null) {
      return null;
    }
    const control = await getAutomationEnrollmentControl(
      address.projectId,
      address.issueId,
    );
    if (!control.managed || control.terminated_at != null) {
      return null;
    }
    const binding = control.binding ?? null;
    const stopKey = control.stop_key ?? null;
    if (binding === null || stopKey === null) {
      return null;
    }
    if (!control.allowed_actions.includes("continue_plan")) {
      return null;
    }
    return {
      projectId: address.projectId,
      issueId: address.issueId,
      binding,
      stopKey,
      contextRequired: control.context_required,
    };
  } catch {
    // 读取失败（断网/4xx/形状异常）保持无动作面，不伪造可用状态。
    return null;
  }
}

function describeContinueFailure(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    const code =
      "code" in error && typeof error.code === "string" && error.code !== ""
        ? error.code
        : null;
    const message =
      "message" in error && typeof error.message === "string" && error.message !== ""
        ? error.message
        : null;
    if (code !== null) {
      return message !== null ? `${code} · ${message}` : code;
    }
  }
  return error instanceof Error && error.message !== ""
    ? error.message
    : "继续自动化请求失败";
}

export function ManagedStoppedContinueAction({
  sessionId,
}: {
  /** stopped 条目所属 workspace session（卡片 id 前缀解析而来；null 不出面）。 */
  sessionId: string | null;
}) {
  const [facts, setFacts] = useState<ContinueFacts | null>(null);
  const [composerOpen, setComposerOpen] = useState(false);
  const [context, setContext] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [notice, setNotice] = useState<ContinueNotice | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refreshFacts = useCallback(async (): Promise<ContinueFacts | null> => {
    if (sessionId === null) {
      setFacts(null);
      return null;
    }
    const next = await loadContinueFacts(sessionId);
    if (mountedRef.current) {
      setFacts(next);
    }
    return next;
  }, [sessionId]);

  // 打开卡片即拉取事实（打开/点击口径之一），服务端才是权威。
  useEffect(() => {
    void refreshFacts();
  }, [refreshFacts]);

  if (sessionId === null) {
    return null;
  }
  const trimmed = context.trim();
  const contextMissing = facts !== null && facts.contextRequired && trimmed.length === 0;
  // 无可用事实且无既有回执 → 不占高度（指引文案由卡片自身承载）。
  if (facts === null && notice === null) {
    return null;
  }

  const handleOpen = () => {
    setNotice(null);
    setComposerOpen(true);
    // 点击时重读（停点可能已漂移）：context_required 取当次事实；若停点已
    // 不再允许继续，收起输入并如实说明（不留下无法提交的表单）。
    void refreshFacts().then((fresh) => {
      if (!mountedRef.current || fresh !== null) {
        return;
      }
      setComposerOpen(false);
      setNotice({
        tone: "danger",
        text: "automation_continue_not_allowed · 当前停点不允许继续（allowed_actions 不含 continue_plan）",
      });
    });
  };

  const handleCancel = () => {
    setComposerOpen(false);
    setContext("");
    setNotice(null);
  };

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (sessionId === null || submitting || contextMissing) {
      return;
    }
    setSubmitting(true);
    try {
      // 提交前重读：POST 的身份字段只认当次 GET（不复用旧值、不猜）。
      const fresh = await loadContinueFacts(sessionId);
      if (fresh === null) {
        if (mountedRef.current) {
          setFacts(null);
          setComposerOpen(false);
          setNotice({
            tone: "danger",
            text: "automation_continue_not_allowed · 当前停点不允许继续（allowed_actions 不含 continue_plan）",
          });
        }
        return;
      }
      if (fresh.contextRequired && trimmed.length === 0) {
        // 前端半拦截；服务端 validation 仍是权威兜底。
        if (mountedRef.current) {
          setFacts(fresh);
          setNotice({
            tone: "danger",
            text: "automation_continue_context_required · 本次继续必须填写说明",
          });
        }
        return;
      }
      const status = await postAutomationContinuePlan(fresh.projectId, fresh.issueId, {
        command_id: newCommandId(),
        expected_binding: fresh.binding,
        expected_stop_key: fresh.stopKey,
        ...(trimmed.length > 0 ? { context: trimmed } : {}),
      });
      if (!mountedRef.current) {
        return;
      }
      const dispatched = status.state === "accepted" || status.state === "replayed";
      // 说明随命令被服务端认领：无论收敛还是转分诊都不再保留输入。
      setContext("");
      setFacts(fresh);
      setComposerOpen(!dispatched);
      if (dispatched) {
        setNotice({ tone: "info", text: "已继续，自动化推进中" });
        // 卡片状态刷新走既有 lifecycle invalidation（不新建协议）。
        notifyLifecycleInvalidated(fresh.issueId);
      } else {
        setNotice({
          tone: "warn",
          text: `需人工分诊：${status.reason_code ?? status.state}`,
        });
      }
      // 回执后复读 control：continue_plan 收敛即收起入口。
      void refreshFacts();
    } catch (error) {
      // 409/失败：卡内内联后端 code+message，保留输入，不自动重试。
      if (mountedRef.current) {
        setNotice({ tone: "danger", text: describeContinueFailure(error) });
      }
    } finally {
      if (mountedRef.current) {
        setSubmitting(false);
      }
    }
  };

  return (
    <div data-testid="stopped-managed-continue" className="mt-2 space-y-2">
      {notice !== null ? (
        <p
          data-testid="stopped-managed-continue-notice"
          role={notice.tone === "danger" ? "alert" : "status"}
          className={[
            "aria-mono rounded-md border px-2.5 py-1.5 text-xs leading-4",
            notice.tone === "info"
              ? "border-emerald-200 bg-emerald-50 text-emerald-800"
              : notice.tone === "warn"
                ? "border-amber-200 bg-amber-50 text-amber-900"
                : "border-red-200 bg-red-50 text-red-700",
          ].join(" ")}
        >
          {notice.text}
        </p>
      ) : null}
      {facts !== null && !composerOpen ? (
        <button
          type="button"
          data-testid="stopped-managed-continue-open"
          onClick={handleOpen}
          className="inline-flex min-h-9 items-center gap-1 rounded-md border border-amber-300 bg-white px-2.5 py-1 text-xs font-semibold text-amber-900 transition-colors duration-200 hover:bg-amber-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-500"
        >
          <RefreshCcw className="h-3.5 w-3.5" aria-hidden="true" />
          补说明并继续
        </button>
      ) : null}
      {facts !== null && composerOpen ? (
        <form
          data-testid="stopped-managed-continue-form"
          onSubmit={(event) => {
            void handleSubmit(event);
          }}
          className="space-y-2"
        >
          <textarea
            data-testid="stopped-managed-continue-context"
            value={context}
            onChange={(event) => setContext(event.target.value)}
            rows={3}
            placeholder={
              facts.contextRequired
                ? "用一两句话说明情况；该说明会随「继续自动化」一并带给 AI"
                : "可补充说明（可选），或直接提交继续"
            }
            className="min-h-20 w-full resize-y rounded-md border border-amber-200 bg-white px-3 py-2 text-sm text-slate-900 placeholder:text-slate-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-500"
          />
          {contextMissing ? (
            <p
              data-testid="stopped-managed-continue-context-hint"
              className="text-xs font-semibold text-[var(--aria-danger)]"
            >
              请先写一两句话说明情况——该说明会随「继续自动化」一并带给 AI
            </p>
          ) : null}
          <div className="flex flex-wrap gap-2">
            <button
              type="submit"
              data-testid="stopped-managed-continue-submit"
              disabled={submitting || contextMissing}
              className={`${BTN_PRIMARY_CLASS} disabled:cursor-not-allowed disabled:opacity-60`}
            >
              {submitting ? "处理中" : "提交"}
            </button>
            <button
              type="button"
              data-testid="stopped-managed-continue-cancel"
              onClick={handleCancel}
              disabled={submitting}
              className={`${BTN_SECONDARY_CLASS} disabled:cursor-not-allowed disabled:opacity-60`}
            >
              取消
            </button>
          </div>
        </form>
      ) : null}
    </div>
  );
}
