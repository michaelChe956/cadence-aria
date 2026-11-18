// T5 驾驶舱「日志打包」入口：设置旁按钮 + 授权下载弹窗。
// 流程：仅告知 → 申请授权（POST /api/diagnostics/log-bundle/authorize，same-origin）
// → 展示宿主终端批准命令（只含申请号；下载句柄只驻浏览器内存 ref，
// 不进 DOM/URL/持久存储）→ 显式确认下载（POST JSON {confirmed:true,handle}）
// → 仅 response.ok + gzip MIME + 非空 Blob 才用带时间戳的 <a download> 触发附件
// 并回收 object URL；取消/过期/未批准/繁忙/网络异常对用户可见；
// 对话框取消或组件卸载后不误触浏览器下载。
import { useCallback, useEffect, useRef, useState, type JSX } from "react";
import { Package } from "lucide-react";
import { DialogShell } from "../shared/DialogShell";

const AUTHORIZE_ENDPOINT = "/api/diagnostics/log-bundle/authorize";
const DOWNLOAD_ENDPOINT = "/api/diagnostics/log-bundle/download";
// 申请号固定 32 位小写 hex，授权有效期固定 300 秒；不一致即视为异常响应。
const REQUEST_ID_PATTERN = /^[0-9a-f]{32}$/;
const EXPECTED_EXPIRES_IN_SECONDS = 300;

type AuthorizePayload = {
  request_id: string;
  handle: string;
  expires_in_seconds: number;
};

type FlowPhase =
  | "notice"
  | "requesting"
  | "awaiting"
  | "downloading"
  | "downloaded";

const PRIMARY_BUTTON_CLASS =
  "inline-flex h-9 cursor-pointer items-center justify-center rounded-lg bg-[var(--aria-primary)] px-4 text-sm font-medium text-[var(--aria-bg)] transition-colors duration-200 hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50";

const SECONDARY_BUTTON_CLASS =
  "inline-flex h-9 cursor-pointer items-center justify-center rounded-lg border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-4 text-sm text-[var(--aria-ink-muted)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50";

function isAuthorizePayload(value: unknown): value is AuthorizePayload {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record.request_id === "string" &&
    REQUEST_ID_PATTERN.test(record.request_id) &&
    typeof record.handle === "string" &&
    record.handle.length > 0 &&
    record.expires_in_seconds === EXPECTED_EXPIRES_IN_SECONDS
  );
}

function downloadFileName(now: Date = new Date()): string {
  const pad = (value: number): string => String(value).padStart(2, "0");
  const stamp =
    `${now.getUTCFullYear()}${pad(now.getUTCMonth() + 1)}${pad(now.getUTCDate())}` +
    `-${pad(now.getUTCHours())}${pad(now.getUTCMinutes())}${pad(now.getUTCSeconds())}`;
  return `aria-diagnostics-${stamp}.tar.gz`;
}

export function CockpitLogBundleButton(): JSX.Element {
  const [open, setOpen] = useState(false);
  const [phase, setPhase] = useState<FlowPhase>("notice");
  const [requestId, setRequestId] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  // 句柄只驻浏览器内存：不渲染、不进 URL、不落任何持久存储。
  const handleRef = useRef<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const inflightRef = useRef(false);

  useEffect(
    () => () => {
      abortRef.current?.abort();
    },
    [],
  );

  const beginAttempt = useCallback((): AbortController => {
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    return controller;
  }, []);

  const closeDialog = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setOpen(false);
    setStatusMessage(null);
    if (phase === "downloaded") {
      setPhase("notice");
      setRequestId(null);
      handleRef.current = null;
    }
  }, [phase]);

  const requestAuthorization = useCallback(async (): Promise<void> => {
    if (inflightRef.current) {
      return;
    }
    inflightRef.current = true;
    setStatusMessage(null);
    setPhase("requesting");
    const controller = beginAttempt();
    try {
      const response = await fetch(AUTHORIZE_ENDPOINT, {
        method: "POST",
        signal: controller.signal,
      });
      if (!response.ok) {
        setPhase("notice");
        setStatusMessage("授权申请被服务拒绝，请稍后重试。");
        return;
      }
      const contentType = response.headers.get("content-type") ?? "";
      if (!contentType.toLowerCase().includes("json")) {
        setPhase("notice");
        setStatusMessage("授权服务返回了无法识别的内容，未继续。");
        return;
      }
      const payload: unknown = await response.json();
      if (!isAuthorizePayload(payload)) {
        setPhase("notice");
        setStatusMessage("授权服务响应不完整，未继续。");
        return;
      }
      handleRef.current = payload.handle;
      setRequestId(payload.request_id);
      setPhase("awaiting");
    } catch {
      if (controller.signal.aborted) {
        return;
      }
      setPhase("notice");
      setStatusMessage("网络异常，授权申请未完成，请重试。");
    } finally {
      inflightRef.current = false;
    }
  }, [beginAttempt]);

  const confirmDownload = useCallback(async (): Promise<void> => {
    const handle = handleRef.current;
    if (handle === null || inflightRef.current) {
      return;
    }
    inflightRef.current = true;
    setStatusMessage(null);
    setPhase("downloading");
    const controller = beginAttempt();
    try {
      const response = await fetch(DOWNLOAD_ENDPOINT, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ confirmed: true, handle }),
        signal: controller.signal,
      });
      if (!response.ok) {
        setPhase("awaiting");
        setStatusMessage(
          "下载被服务拒绝：若尚未批准，请在宿主终端完成 aria diagnostics approve 后重试；若授权已过期，请重新申请授权。",
        );
        return;
      }
      const contentType = response.headers.get("content-type") ?? "";
      if (!contentType.toLowerCase().includes("gzip")) {
        setPhase("awaiting");
        setStatusMessage("服务返回的不是 gzip 诊断包，已取消保存。");
        return;
      }
      const blob = await response.blob();
      if (blob.size === 0) {
        setPhase("awaiting");
        setStatusMessage("服务返回了空的诊断包，已取消保存。");
        return;
      }
      if (controller.signal.aborted) {
        // 对话框已取消或组件已卸载：不触发浏览器下载。
        return;
      }
      const objectUrl = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = objectUrl;
      anchor.download = downloadFileName();
      anchor.rel = "noopener";
      document.body.appendChild(anchor);
      anchor.click();
      document.body.removeChild(anchor);
      window.setTimeout(() => URL.revokeObjectURL(objectUrl), 0);
      setPhase("downloaded");
    } catch {
      if (controller.signal.aborted) {
        return;
      }
      setPhase("awaiting");
      setStatusMessage("下载未完成（网络异常），请重试或重新申请授权。");
    } finally {
      inflightRef.current = false;
    }
  }, [beginAttempt]);

  const showApproval = phase === "awaiting" || phase === "downloading";

  return (
    <>
      <button
        type="button"
        data-testid="cockpit-log-bundle-trigger"
        aria-label="日志打包"
        title="日志打包"
        onClick={() => setOpen(true)}
        className="inline-flex h-11 w-11 cursor-pointer items-center justify-center rounded-xl border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] shadow-md transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
      >
        <Package aria-hidden="true" className="h-4 w-4" />
      </button>
      {open ? (
        <DialogShell
          label="日志打包"
          title="日志打包"
          icon={<Package aria-hidden="true" className="h-4 w-4" />}
          onClose={closeDialog}
          closeLabel="关闭"
          layerClass="z-[110]"
          widthClass="max-w-xl"
        >
          <div className="space-y-4 text-sm text-[var(--aria-ink)]">
            <section className="space-y-2">
              <h3 className="text-sm font-semibold">将收集的内容</h3>
              <ul className="list-disc space-y-1 pl-5">
                <li>仅针对运行 Aria 的宿主账号本机数据，由宿主操作者授权；浏览器访客身份不能代替。</li>
                <li>宿主 $HOME/.aria/projects 下 aggregate-initializations 与 aggregate-recipe-receipts 两类 JSON。</li>
                <li>Claude Code、Codex、Pi、Kimi Code 四家各最近 3 个会话的诊断轨迹。</li>
                <li>当前运行信息 runtime-info.json（含绝对 workspace_root 工作区路径）。</li>
              </ul>
              <p>会话仅导出白名单诊断轨迹（时间、类型、模型、工具类型、退出码、错误类型），不含对话、源码或命令正文；无法解析的内容不会原文写入。</p>
              <p>运行信息中的绝对工作区路径与初始化记录仍可能包含项目或命令详情；Aria 不会向任何第三方上传，请下载后先检查内容，再自行分享。</p>
            </section>

            {showApproval && requestId !== null ? (
              <section className="space-y-2">
                <h3 className="text-sm font-semibold">宿主批准</h3>
                <code
                  data-testid="cockpit-log-bundle-approve-command"
                  className="block break-all rounded-lg border border-[var(--aria-line-strong)] bg-[var(--aria-panel-muted)] px-3 py-2 font-mono text-xs text-[var(--aria-ink)]"
                >
                  aria diagnostics approve {requestId}
                </code>
                <p>请在宿主终端核对命令与来源后执行批准；授权自申请起 5 分钟内有效，逾期需重新申请。</p>
                <p>若用 npx 启动 Aria，将命令中的 aria 换成 npx @cadence-aria/cli。</p>
              </section>
            ) : null}

            {phase === "requesting" ? (
              <p role="status">正在申请授权…</p>
            ) : null}
            {phase === "downloading" ? (
              <p role="status">正在生成诊断包…</p>
            ) : null}
            {phase === "downloaded" ? (
              <p role="status">诊断包已开始下载（文件名含时间戳），请检查内容后再自行分享。</p>
            ) : null}
            {statusMessage !== null ? (
              <p
                role="alert"
                data-testid="cockpit-log-bundle-status"
                className="rounded-lg border border-[var(--aria-danger)] px-3 py-2 text-[var(--aria-danger)]"
              >
                {statusMessage}
              </p>
            ) : null}

            <div className="flex flex-wrap items-center justify-end gap-2">
              {phase === "downloaded" ? (
                <button
                  type="button"
                  className={PRIMARY_BUTTON_CLASS}
                  onClick={closeDialog}
                >
                  完成
                </button>
              ) : (
                <>
                  {phase === "notice" || phase === "requesting" ? (
                    <button
                      type="button"
                      className={PRIMARY_BUTTON_CLASS}
                      disabled={phase === "requesting"}
                      onClick={() => void requestAuthorization()}
                    >
                      申请授权
                    </button>
                  ) : (
                    <>
                      <button
                        type="button"
                        className={PRIMARY_BUTTON_CLASS}
                        disabled={phase === "downloading"}
                        onClick={() => void confirmDownload()}
                      >
                        确认并下载
                      </button>
                      <button
                        type="button"
                        className={SECONDARY_BUTTON_CLASS}
                        disabled={phase === "downloading"}
                        onClick={() => void requestAuthorization()}
                      >
                        重新申请授权
                      </button>
                    </>
                  )}
                  <button
                    type="button"
                    className={SECONDARY_BUTTON_CLASS}
                    onClick={closeDialog}
                  >
                    取消
                  </button>
                </>
              )}
            </div>
          </div>
        </DialogShell>
      ) : null}
    </>
  );
}
