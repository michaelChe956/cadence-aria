// provider-install-update（块2 片7B）：安装/更新按钮的显示规则与异步作业轮询。
//
// - REQ-PCI-02 显示规则：未安装 → 【安装】+ 手动命令；已安装未核验 → 【核验】
//   （不显示安装）；版本落后 → 【更新】+ 当前 vs 最新；最新 → 不渲染任何按钮。
// - REQ-PCI-01：Kimi Code 走官方脚本，点击前必须告知「将执行来自 code.kimi.com
//   的安装脚本」；未确认不得调 API（POST 带 acknowledge_remote_script=true）。
// - REQ-PCI-04：Kimi 不做版本判定 → 永不出现「更新」。
// - REQ-PCI-05：POST 立即返回 job_id → 立刻显示运行中；轮询到终态后回调
//   onSettled 并停止轮询（作业状态是 durable 的，刷新不丢）。
// - REQ-PCI-06：失败作业如实展示分类提示 + 手动命令。
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ProviderInstallActionButton,
  type ProviderInstallActionButtonProps,
} from "./ProviderInstallActionButton";
import type { InstallJob, InstallTargetWire } from "../../api/types";

const NPM_MANUAL = (pkg: string): string =>
  `npm install -g --registry=https://registry.npmmirror.com ${pkg}`;

type CapturedRequest = { url: string; method: string; body: unknown };

type JobResponder = () => InstallJob;

/** POST /api/providers/{target}/install + GET /api/providers/install/{job_id}
 * 的真实发送器路由；job 应答按序列依次出队（默认一直返回 running）。 */
function installFetchRouter(options: {
  target: InstallTargetWire;
  jobIds?: string[];
  jobs?: JobResponder[];
}): { requests: CapturedRequest[] } {
  const requests: CapturedRequest[] = [];
  const jobIds = [...(options.jobIds ?? ["job_0001"])];
  const jobs = [...(options.jobs ?? [])];
  const router = async (
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    const url = typeof input === "string" ? input : input.toString();
    const method = (init?.method ?? "GET").toUpperCase();
    const rawBody = typeof init?.body === "string" ? init.body : null;
    requests.push({
      url,
      method,
      body: rawBody === null ? null : JSON.parse(rawBody),
    });
    if (
      method === "POST" &&
      url === `/api/providers/${options.target}/install`
    ) {
      return jsonResponse({ job_id: jobIds.shift() ?? "job_0001" });
    }
    const jobMatch = /^\/api\/providers\/install\/(.+)$/.exec(url);
    if (method === "GET" && jobMatch) {
      const responder = jobs.length > 1 ? jobs.shift()! : jobs[0];
      const job = responder
        ? responder()
        : runningJob(jobMatch[1], options.target);
      return jsonResponse(job);
    }
    return jsonResponse(
      { code: "not_found", message: `no route for ${method} ${url}` },
      404,
    );
  };
  vi.stubGlobal("fetch", vi.fn(router));
  return { requests };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function runningJob(jobId: string, target: InstallTargetWire): InstallJob {
  return {
    job_id: jobId,
    target,
    status: "running",
    created_at: "2026-11-03T00:00:00Z",
    updated_at: "2026-11-03T00:00:00Z",
    outcome: null,
    failure: null,
  };
}

function completedJob(jobId: string, target: InstallTargetWire): InstallJob {
  return {
    ...runningJob(jobId, target),
    status: "completed",
    outcome: { installed_version: "1.0.34", path_hint: "~/.npm-global/bin" },
  };
}

function failedJob(jobId: string, target: InstallTargetWire): InstallJob {
  return {
    ...runningJob(jobId, target),
    status: "failed",
    failure: {
      kind: "permission",
      message: "装不上（权限不足）。请用管理员权限运行下面的命令。",
      manual_command: NPM_MANUAL("@openai/codex"),
    },
  };
}

function renderButton(
  overrides: Partial<ProviderInstallActionButtonProps> = {},
) {
  return render(
    <ProviderInstallActionButton
      target="codex"
      cliAvailable={false}
      capabilityConfirmed={false}
      {...overrides}
    />,
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("ProviderInstallActionButton 显示规则（REQ-PCI-02）", () => {
  it("未安装时渲染「安装」与手动命令，且不渲染「更新」", () => {
    installFetchRouter({ target: "codex" });
    renderButton({ cliAvailable: false });

    expect(screen.getByRole("button", { name: "安装" })).toBeInTheDocument();
    expect(
      screen.getByTestId("provider-install-manual-command-codex"),
    ).toHaveTextContent(NPM_MANUAL("@openai/codex"));
    expect(
      screen.queryByRole("button", { name: "更新" }),
    ).not.toBeInTheDocument();
  });

  it("已安装但未核验时渲染「核验」，不渲染「安装」", () => {
    installFetchRouter({ target: "codex" });
    renderButton({ cliAvailable: true, capabilityConfirmed: false });

    expect(
      screen.getByTestId("provider-install-verify-codex"),
    ).toHaveTextContent("核验");
    expect(
      screen.queryByTestId("provider-install-start-codex"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "安装" }),
    ).not.toBeInTheDocument();
  });

  it("已安装已核验且版本落后时渲染「更新」与当前/最新版本", () => {
    installFetchRouter({ target: "codex" });
    renderButton({
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "0.85.0",
      latestVersion: "1.5.0",
    });

    expect(screen.getByRole("button", { name: "更新" })).toBeInTheDocument();
    expect(
      screen.getByTestId("provider-install-versions-codex"),
    ).toHaveTextContent("0.85.0");
    expect(
      screen.getByTestId("provider-install-versions-codex"),
    ).toHaveTextContent("1.5.0");
  });

  it("本地版本等于最新版本时不渲染任何按钮", () => {
    installFetchRouter({ target: "codex" });
    const { container } = renderButton({
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "1.5.0",
      latestVersion: "1.5.0",
    });

    expect(container).toBeEmptyDOMElement();
  });

  it("本地版本高于最新版本时不提供「更新」（不因字符串不等就误报）", () => {
    installFetchRouter({ target: "codex" });
    const { container } = renderButton({
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "1.10.0",
      latestVersion: "1.9.0",
    });

    expect(container).toBeEmptyDOMElement();
  });
});

describe("Kimi Code：安装来源告知与版本判定（REQ-PCI-01 / REQ-PCI-04）", () => {
  it("latestVersion 为 null 时不出现「更新」", () => {
    installFetchRouter({ target: "kimi_code" });
    const { container } = renderButton({
      target: "kimi_code",
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "1.0.0",
      latestVersion: null,
    });

    expect(container).toBeEmptyDOMElement();
    expect(
      screen.queryByRole("button", { name: "更新" }),
    ).not.toBeInTheDocument();
  });

  it("即使给了 latestVersion 也不对 Kimi 做版本判定（永不显示「更新」）", () => {
    installFetchRouter({ target: "kimi_code" });
    const { container } = renderButton({
      target: "kimi_code",
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "1.0.0",
      latestVersion: "9.9.9",
    });

    expect(container).toBeEmptyDOMElement();
  });

  it("点击安装先告知脚本来源，未确认不调 API；确认后带 acknowledge_remote_script", async () => {
    const { requests } = installFetchRouter({
      target: "kimi_code",
      jobs: [() => runningJob("job_kimi", "kimi_code")],
    });
    renderButton({
      target: "kimi_code",
      cliAvailable: false,
      capabilityConfirmed: false,
    });

    fireEvent.click(screen.getByRole("button", { name: "安装" }));

    expect(
      screen.getByTestId("provider-install-kimi-script-notice-kimi_code"),
    ).toHaveTextContent("将执行来自 code.kimi.com 的安装脚本");
    expect(
      requests.filter((request) => request.method === "POST"),
    ).toHaveLength(0);

    await act(async () => {
      fireEvent.click(screen.getByTestId("provider-install-confirm-kimi_code"));
      await Promise.resolve();
    });

    const posts = requests.filter((request) => request.method === "POST");
    expect(posts).toHaveLength(1);
    expect(posts[0].url).toBe("/api/providers/kimi_code/install");
    expect(posts[0].body).toEqual({ acknowledge_remote_script: true });
  });

  it("未安装时的手动命令是官方脚本命令，不是 npm", () => {
    installFetchRouter({ target: "kimi_code" });
    renderButton({
      target: "kimi_code",
      cliAvailable: false,
      capabilityConfirmed: false,
    });

    expect(
      screen.getByTestId("provider-install-manual-command-kimi_code"),
    ).toHaveTextContent(
      "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash",
    );
  });
});

describe("异步安装作业（REQ-PCI-05 / REQ-PCI-06）", () => {
  it("点击后立刻显示运行中，轮询到 completed 后调 onSettled 并停止轮询", async () => {
    vi.useFakeTimers();
    const onSettled = vi.fn();
    const { requests } = installFetchRouter({
      target: "codex",
      jobIds: ["job_0001"],
      jobs: [
        () => runningJob("job_0001", "codex"),
        () => completedJob("job_0001", "codex"),
      ],
    });
    renderButton({ cliAvailable: false, onSettled });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "安装" }));
      await Promise.resolve();
    });

    // POST 已发出并拿到 job_id → 立刻显示运行中（不等轮询）。
    expect(
      screen.getByTestId("provider-install-status-codex"),
    ).toHaveTextContent("安装中");
    expect(onSettled).not.toHaveBeenCalled();

    // 第一轮轮询：仍在运行。
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(onSettled).not.toHaveBeenCalled();

    // 第二轮轮询：completed → 回调 + 终态文案。
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(
      screen.getByTestId("provider-install-status-codex"),
    ).toHaveTextContent("已安装");
    expect(onSettled).toHaveBeenCalledTimes(1);

    const getCount = () =>
      requests.filter(
        (request) =>
          request.method === "GET" &&
          request.url === "/api/providers/install/job_0001",
      ).length;
    expect(getCount()).toBe(2);

    // 终态后不再轮询（timer 已清）。
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(getCount()).toBe(2);
    expect(onSettled).toHaveBeenCalledTimes(1);
  });

  it("作业失败时展示分类提示与手动命令，不谎报成功", async () => {
    vi.useFakeTimers();
    const onSettled = vi.fn();
    installFetchRouter({
      target: "codex",
      jobIds: ["job_fail"],
      jobs: [() => failedJob("job_fail", "codex")],
    });
    renderButton({ cliAvailable: false, onSettled });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "安装" }));
      await Promise.resolve();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });

    const status = screen.getByTestId("provider-install-status-codex");
    expect(status).toHaveTextContent("权限不足");
    expect(status).toHaveTextContent(NPM_MANUAL("@openai/codex"));
    expect(status).not.toHaveTextContent("已安装");
    expect(onSettled).toHaveBeenCalledTimes(1);
  });
});
