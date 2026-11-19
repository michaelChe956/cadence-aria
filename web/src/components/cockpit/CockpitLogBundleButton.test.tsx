import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CockpitLogBundleButton } from "./CockpitLogBundleButton";

const AUTHORIZE_URL = "/api/diagnostics/log-bundle/authorize";
const DOWNLOAD_URL = "/api/diagnostics/log-bundle/download";
const REQUEST_ID = "0123456789abcdef0123456789abcdef";
// 句柄绝不能出现在界面/DOM；用它作为泄漏探针。
const HANDLE = "handle-must-stay-out-of-dom-4d1c9f";

const fetchMock = vi.fn();
const createObjectURLMock = vi.fn(() => "blob:aria-log-bundle-test");
const revokeObjectURLMock = vi.fn();
let clickedAnchor: HTMLAnchorElement | null = null;

const anchorClick = vi
  .spyOn(HTMLAnchorElement.prototype, "click")
  .mockImplementation(function (this: HTMLAnchorElement) {
    clickedAnchor = this;
  });

function authorizeOk(overrides: Record<string, unknown> = {}): Response {
  return new Response(
    JSON.stringify({
      request_id: REQUEST_ID,
      handle: HANDLE,
      expires_in_seconds: 300,
      ...overrides,
    }),
    { status: 200, headers: { "content-type": "application/json" } },
  );
}

function htmlPage(): Response {
  return new Response("<!doctype html><html><body>proxy login</body></html>", {
    status: 200,
    headers: { "content-type": "text/html; charset=utf-8" },
  });
}

function gzipBundle(bytes = 32): Response {
  return new Response(new Uint8Array(bytes), {
    status: 200,
    headers: { "content-type": "application/gzip" },
  });
}

async function openDialog(): Promise<void> {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "日志打包" }));
}

async function grantAuthorization(): Promise<void> {
  const user = userEvent.setup();
  fetchMock.mockResolvedValueOnce(authorizeOk());
  await user.click(screen.getByRole("button", { name: "申请授权" }));
  await screen.findByTestId("cockpit-log-bundle-approve-command");
}

beforeEach(() => {
  fetchMock.mockReset();
  clickedAnchor = null;
  createObjectURLMock.mockClear();
  revokeObjectURLMock.mockClear();
  vi.stubGlobal("fetch", fetchMock);
  vi.stubGlobal("URL", {
    ...URL,
    createObjectURL: createObjectURLMock,
    revokeObjectURL: revokeObjectURLMock,
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function deferredResponse() {
  let resolve!: (response: Response) => void;
  const promise = new Promise<Response>((settle) => { resolve = settle; });
  return { promise, resolve };
}

describe("CockpitLogBundleButton", () => {
  it("首次打开仅展示告知内容，不发起授权或下载请求", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();

    expect(screen.getByText("将收集的内容")).toBeInTheDocument();
    expect(screen.getByText(/运行 Aria 的宿主账号/)).toBeInTheDocument();
    expect(screen.getByText(/两类 JSON/)).toBeInTheDocument();
    expect(screen.getByText(/四家各最近 3 个会话/)).toBeInTheDocument();
    expect(screen.getByText(/不含对话、源码或命令正文/)).toBeInTheDocument();
    expect(screen.getByText(/workspace_root/)).toBeInTheDocument();
    expect(screen.getByText(/自行分享/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "申请授权" }),
    ).toBeEnabled();
    expect(
      screen.queryByTestId("cockpit-log-bundle-approve-command"),
    ).not.toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("打开后取消关闭：不读取、不采集、无任何请求", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    await user.click(screen.getByRole("button", { name: "取消" }));

    expect(screen.queryByText("将收集的内容")).not.toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("申请授权后展示 npx 批准命令与全局安装提示，句柄不进入界面或 URL", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    await grantAuthorization();

    const command = screen.getByTestId("cockpit-log-bundle-approve-command");
    expect(command.textContent).toBe(
      `npx @cadence-aria/cli diagnostics approve ${REQUEST_ID}`,
    );
    expect(screen.getByText(/宿主终端/)).toBeInTheDocument();
    expect(screen.getByText(/5 分钟/)).toBeInTheDocument();
    expect(screen.getByText(/全局安装/)).toBeInTheDocument();
    expect(
      screen.getByText(/aria diagnostics approve <申请号>/),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain(HANDLE);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [authorizeUrl, authorizeInit] = fetchMock.mock
      .calls[0] as unknown as [string, RequestInit];
    expect(authorizeUrl).toBe(AUTHORIZE_URL);
    expect(authorizeUrl).not.toContain(HANDLE);
    expect(authorizeInit?.method).toBe("POST");
    expect(
      screen.getByRole("button", { name: "确认并下载" }),
    ).toBeEnabled();
  });

  it("授权接口返回 HTML 时不进入下一步并显示错误", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    fetchMock.mockResolvedValueOnce(htmlPage());
    await user.click(screen.getByRole("button", { name: "申请授权" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("授权");
    expect(
      screen.queryByTestId("cockpit-log-bundle-approve-command"),
    ).not.toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("button", { name: "申请授权" }),
    ).toBeEnabled();
  });

  it("授权响应申请号不合法时不展示批准命令", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    fetchMock.mockResolvedValueOnce(
      authorizeOk({ request_id: "NOT-A-VALID-HEX-REQUEST-ID" }),
    );
    await user.click(screen.getByRole("button", { name: "申请授权" }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(
      screen.queryByTestId("cockpit-log-bundle-approve-command"),
    ).not.toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("未批准时下载被拒绝且不保存附件，批准后重试成功", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    await grantAuthorization();

    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ error: "not_approved" }), {
        status: 403,
        headers: { "content-type": "application/json" },
      }),
    );
    await user.click(screen.getByRole("button", { name: "确认并下载" }));

    const status = await screen.findByRole("alert");
    expect(status.textContent).toContain("批准");
    expect(createObjectURLMock).not.toHaveBeenCalled();
    expect(clickedAnchor).toBeNull();
    expect(
      screen.getByRole("button", { name: "确认并下载" }),
    ).toBeEnabled();

    fetchMock.mockResolvedValueOnce(gzipBundle());
    await user.click(screen.getByRole("button", { name: "确认并下载" }));
    await waitFor(() => expect(clickedAnchor).not.toBeNull());
  });

  it("确认下载发送显式确认体，服务返回 HTML 时不误存附件", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    await grantAuthorization();

    fetchMock.mockResolvedValueOnce(htmlPage());
    await user.click(screen.getByRole("button", { name: "确认并下载" }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    const [downloadUrl, downloadInit] = fetchMock.mock
      .calls[1] as unknown as [string, RequestInit];
    expect(downloadUrl).toBe(DOWNLOAD_URL);
    expect(downloadUrl).not.toContain(HANDLE);
    expect(downloadInit?.method).toBe("POST");
    expect(JSON.parse(String(downloadInit?.body))).toEqual({
      confirmed: true,
      handle: HANDLE,
    });
    expect(createObjectURLMock).not.toHaveBeenCalled();
    expect(clickedAnchor).toBeNull();
  });

  it("下载返回空 gzip 包时不保存附件", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    await grantAuthorization();

    fetchMock.mockResolvedValueOnce(gzipBundle(0));
    await user.click(screen.getByRole("button", { name: "确认并下载" }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(createObjectURLMock).not.toHaveBeenCalled();
    expect(clickedAnchor).toBeNull();
  });

  it("明确授权确认后生成带时间戳的下载并回收 object URL", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    await grantAuthorization();

    fetchMock.mockResolvedValueOnce(gzipBundle());
    await user.click(screen.getByRole("button", { name: "确认并下载" }));

    await waitFor(() => expect(clickedAnchor).not.toBeNull());
    expect(clickedAnchor?.download).toMatch(
      /^aria-diagnostics-\d{8}-\d{6}\.tar\.gz$/,
    );
    expect(clickedAnchor?.href).toBe("blob:aria-log-bundle-test");
    await waitFor(() =>
      expect(revokeObjectURLMock).toHaveBeenCalledWith(
        "blob:aria-log-bundle-test",
      ),
    );
    expect(screen.getByRole("button", { name: "完成" })).toBeEnabled();
  });

  it("授权请求进行中呈现繁忙且不重复发起", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    const authorizeDeferred = deferredResponse();
    fetchMock.mockImplementationOnce(() => authorizeDeferred.promise);
    await user.click(screen.getByRole("button", { name: "申请授权" }));

    expect(screen.getByText("正在申请授权…")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "申请授权" }),
    ).toBeDisabled();

    await act(async () => {
      authorizeDeferred.resolve(authorizeOk());
    });
    await screen.findByTestId("cockpit-log-bundle-approve-command");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("下载进行中取消关闭后不触发浏览器下载", async () => {
    const user = userEvent.setup();
    render(<CockpitLogBundleButton />);

    await openDialog();
    await grantAuthorization();

    const downloadDeferred = deferredResponse();
    fetchMock.mockImplementationOnce(() => downloadDeferred.promise);
    await user.click(screen.getByRole("button", { name: "确认并下载" }));
    await user.click(screen.getByRole("button", { name: "取消" }));

    await act(async () => {
      downloadDeferred.resolve(gzipBundle());
    });
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 30));
    });

    expect(clickedAnchor).toBeNull();
    expect(createObjectURLMock).not.toHaveBeenCalled();
  });
});
