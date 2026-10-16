// 批2 Task 1：控制面认证前端闭环（REQ-API-AUTH-01/02）。
//
// 合同（plan 批量2 T1）：
// - 认证入口先查会话（GET /api/auth/session，204=已认证/401=未认证），
//   未认证时展示一次性 bootstrap token 输入，经 POST /api/auth/session 仅以
//   body 交换（token 绝不出现在 URL/localStorage）。
// - 成功后 cookie 由服务端 HttpOnly Set-Cookie 建立；前端刷新直接复用 cookie。
// - 任何 API 请求返回 401 时经 AUTH_REQUIRED_EVENT 回到认证态，
//   绝不自动重发副作用请求。

import { useEffect, useState, type ReactNode } from "react";

import { ApiRequestError, AUTH_REQUIRED_EVENT, normalizeApiError } from "./client";

export { AUTH_REQUIRED_EVENT };

export type SessionStatus = "checking" | "unauthenticated" | "authenticated";

/** 查询当前会话状态：204 → true；401 → false；其它错误抛 ApiRequestError。 */
export async function checkSession(): Promise<boolean> {
  const response = await fetch("/api/auth/session", {
    credentials: "same-origin",
  });
  if (response.status === 204) {
    return true;
  }
  if (response.status === 401) {
    return false;
  }
  throw new ApiRequestError(await normalizeApiError(response));
}

/** 用一次性 bootstrap token 交换会话（服务端经 Set-Cookie 下发 aria_session）。 */
export async function exchangeBootstrapToken(token: string): Promise<void> {
  const response = await fetch("/api/auth/session", {
    method: "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ token }),
  });
  if (response.ok) {
    return;
  }
  throw new ApiRequestError(await normalizeApiError(response));
}


/** 认证入口：输入 owner 终端交付的一次性 token 并提交。 */
export function BootstrapAuthEntry({
  onAuthenticated,
}: {
  onAuthenticated: () => void;
}) {
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    const trimmed = token.trim();
    if (!trimmed || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await exchangeBootstrapToken(trimmed);
      onAuthenticated();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <main className="auth-gate" aria-label="控制面认证">
      <h1>Aria 控制面认证</h1>
      <p>
        平台 API 需要认证会话。请粘贴启动终端显示的一次性 bootstrap token
        （仅本次进程有效，不会再次显示）。
      </p>
      <form onSubmit={submit}>
        <label htmlFor="aria-bootstrap-token">一次性 Bootstrap Token</label>
        <input
          id="aria-bootstrap-token"
          name="aria-bootstrap-token"
          type="password"
          autoComplete="off"
          value={token}
          onChange={(event) => setToken(event.target.value)}
          placeholder="64 位十六进制 token"
        />
        <button type="submit" disabled={submitting || token.trim().length === 0}>
          {submitting ? "认证中…" : "认证"}
        </button>
      </form>
      {error ? (
        <p role="alert" className="auth-gate-error">
          {error}
        </p>
      ) : null}
    </main>
  );
}

/** 认证门：先查会话/交换 token，再挂载 AppRouter；401 事件回到认证态。 */
export function AuthGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<SessionStatus>("checking");

  useEffect(() => {
    let cancelled = false;
    checkSession()
      .then((authenticated) => {
        if (!cancelled) {
          setStatus(authenticated ? "authenticated" : "unauthenticated");
        }
      })
      .catch(() => {
        if (!cancelled) {
          setStatus("unauthenticated");
        }
      });
    const onAuthRequired = () => setStatus("unauthenticated");
    window.addEventListener(AUTH_REQUIRED_EVENT, onAuthRequired);
    return () => {
      cancelled = true;
      window.removeEventListener(AUTH_REQUIRED_EVENT, onAuthRequired);
    };
  }, []);

  if (status === "checking") {
    return null;
  }
  if (status === "unauthenticated") {
    return <BootstrapAuthEntry onAuthenticated={() => setStatus("authenticated")} />;
  }
  return <>{children}</>;
}
