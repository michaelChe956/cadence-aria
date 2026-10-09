/// 只读 GET 客户端:backend-dump 与段首前置核对共用。
/// 纪律:本冒烟线不通过 HTTP 发任何业务变更请求——建档/登记/初始化/建 issue
/// 全部由浏览器真实点击完成;HTTP 面仅限 GET 只读观测。

export type ApiReadResult = { status: number; body: unknown; at: string };

export async function apiGet(baseURL: string, pathname: string, timeoutMs = 10_000): Promise<ApiReadResult> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(`${baseURL}${pathname}`, {
      method: "GET",
      headers: { accept: "application/json" },
      signal: controller.signal,
    });
    const text = await response.text();
    let body: unknown = null;
    if (text.trim().length > 0) {
      try {
        body = JSON.parse(text);
      } catch {
        body = { raw: text.slice(0, 2000) };
      }
    }
    return { status: response.status, body, at: new Date().toISOString() };
  } finally {
    clearTimeout(timer);
  }
}

/** 拉取产品既有 GET 投影;404 归一为 {status:404},不抛错(dump 需要如实记录)。 */
export async function apiGetSafe(baseURL: string, pathname: string): Promise<ApiReadResult & { error?: string }> {
  try {
    return await apiGet(baseURL, pathname);
  } catch (error) {
    return { status: 0, body: null, at: new Date().toISOString(), error: (error as Error).message };
  }
}
