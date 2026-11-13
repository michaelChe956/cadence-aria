// provider-install-update（块2 片7B）：安装/更新按钮的显示规则与异步作业轮询。
//
// - REQ-PCI-02 显示规则：未安装 → 【安装】+ 手动命令；已安装未核验 → 【核验】
//   （不显示安装）；版本落后 → 【更新】+ 当前 vs 最新；最新 → 不渲染任何按钮。
// - REQ-PCI-01：Kimi Code 走官方脚本，点击前必须告知「将执行来自 code.kimi.com
//   的安装脚本」；未确认不得调 API（POST 带 acknowledge_remote_script=true）。
// - REQ-PCI-07：Kimi 的版本来源是 npm 包 `@moonshot-ai/kimi-code` ⇒ 版本落后时
//   同样出现「更新」（安装源仍是官方脚本 ⇒ 仍先告知来源并确认）。
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

// 后端 npm_path_hint() 返回的是**完整句子**（2026-11-13 确认）：
// 「已安装到 <prefix>；若 `<cmd>` 不在 PATH 上，请把 <prefix>/bin 加入 PATH 后重试」。
const PATH_HINT_SENTENCE =
  "已安装到 /opt/node；若 `codegraph` 不在 PATH 上，请把 /opt/node/bin 加入 PATH 后重试";

function completedJob(jobId: string, target: InstallTargetWire): InstallJob {
  return {
    ...runningJob(jobId, target),
    status: "completed",
    outcome: { installed_version: "1.0.34", path_hint: PATH_HINT_SENTENCE },
  };
}

function completedJobWithOutcome(
  jobId: string,
  target: InstallTargetWire,
  outcome: InstallJob["outcome"],
): InstallJob {
  return {
    ...runningJob(jobId, target),
    status: "completed",
    outcome,
    failure: null,
  };
}

/** 点安装 → 轮询到终态（两轮轮询：running → completed）。 */
async function driveToCompleted(): Promise<void> {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "安装" }));
    await Promise.resolve();
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2_000);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2_000);
  });
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

  // §五 2.11 起：已安装（任意版本）始终渲染【暴露到终端 PATH】，不再
  // 整体返回 null；「安装 / 更新」按钮仍按 REQ-PCI-02 规则不渲染。
  it("本地版本等于最新版本时不渲染「安装/更新」，仅保留「暴露到终端 PATH」", () => {
    installFetchRouter({ target: "codex" });
    renderButton({
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "1.5.0",
      latestVersion: "1.5.0",
    });

    expect(
      screen.queryByRole("button", { name: "安装" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "更新" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByTestId("provider-install-expose-codex"),
    ).toBeInTheDocument();
  });

  it("本地版本高于最新版本时不提供「更新」（不因字符串不等就误报）", () => {
    installFetchRouter({ target: "codex" });
    renderButton({
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "1.10.0",
      latestVersion: "1.9.0",
    });

    expect(
      screen.queryByRole("button", { name: "更新" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByTestId("provider-install-expose-codex"),
    ).toBeInTheDocument();
  });
});

describe("Kimi Code：安装来源告知与版本判定（REQ-PCI-01 / REQ-PCI-07）", () => {
  it("latestVersion 为 null 时不出现「更新」", () => {
    installFetchRouter({ target: "kimi_code" });
    renderButton({
      target: "kimi_code",
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "1.0.0",
      latestVersion: null,
    });

    // §五 2.11：已安装 ⇒ 「暴露到终端 PATH」仍在；「更新」不出现。
    expect(
      screen.getByTestId("provider-install-expose-kimi_code"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "更新" }),
    ).not.toBeInTheDocument();
  });

  // 2026-11-09（REQ-PCI-07）：Kimi Code 的版本来源 = npm 包
  // `@moonshot-ai/kimi-code` ⇒ 参与「有新版」判定，出现【更新】。
  // 但它的**安装源**仍是官方脚本 ⇒ 点击【更新】必须先告知来源并确认。
  it("Kimi 版本落后时显示「更新」与当前/最新版本", () => {
    installFetchRouter({ target: "kimi_code" });
    renderButton({
      target: "kimi_code",
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "2.4.1",
      latestVersion: "2.5.2",
    });

    expect(screen.getByRole("button", { name: "更新" })).toBeInTheDocument();
    expect(
      screen.getByTestId("provider-install-versions-kimi_code"),
    ).toHaveTextContent("2.4.1");
    expect(
      screen.getByTestId("provider-install-versions-kimi_code"),
    ).toHaveTextContent("2.5.2");
  });

  it("点击 Kimi 的「更新」先告知脚本来源，未确认不调 API；确认后带 acknowledge_remote_script", async () => {
    const { requests } = installFetchRouter({
      target: "kimi_code",
      jobs: [() => runningJob("job_kimi", "kimi_code")],
    });
    renderButton({
      target: "kimi_code",
      cliAvailable: true,
      capabilityConfirmed: true,
      localVersion: "2.4.1",
      latestVersion: "2.5.2",
    });

    fireEvent.click(screen.getByRole("button", { name: "更新" }));

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

// 2026-11-09 bug 修复：安装块是多行（按钮行 + 状态文案行），在能力卡的
// 操作区里把「核验」按钮顶歪；长状态文案还把整行撑出卡片。断言：安装块
// 可收缩且不撑宽（min-w-0 / max-w-full），状态文案允许折行（break-words）。
describe("安装块的布局契约（不撑宽、不顶歪操作区）", () => {
  it("安装块可收缩且不撑出所在行", () => {
    installFetchRouter({ target: "codex" });
    renderButton({ cliAvailable: false, capabilityConfirmed: false });

    const action = screen.getByTestId("provider-install-action-codex");
    expect(action.className).toContain("min-w-0");
    expect(action.className).toContain("max-w-full");
  });

  it("运行中状态文案允许折行（不把父行撑宽）", async () => {
    vi.useFakeTimers();
    installFetchRouter({
      target: "codex",
      jobIds: ["job_0001"],
      jobs: [() => runningJob("job_0001", "codex")],
    });
    renderButton({ cliAvailable: false });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "安装" }));
      await Promise.resolve();
    });

    const status = screen.getByTestId("provider-install-status-codex");
    expect(status.className).toContain("break-words");
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

// 2026-11-13 bug 修复（契约变更）：后端 path_hint 本身是完整句子，前端
// 曾再包一层「若命令不在 PATH，可把 X 加入 PATH」⇒ 用户看到破碎双句，
// 误以为「点了安装没效果」。新契约：path_hint **原样渲染**，不二次包裹、
// 不截断；没有 path_hint 就不渲染该提示节点。
describe("安装完成后的 path_hint 呈现（原样渲染后端整句）", () => {
  it("path_hint 是完整句子时逐字渲染，且不再出现二次包裹痕迹", async () => {
    vi.useFakeTimers();
    installFetchRouter({
      target: "codex",
      jobIds: ["job_hint"],
      jobs: [
        () => runningJob("job_hint", "codex"),
        () =>
          completedJobWithOutcome("job_hint", "codex", {
            installed_version: "1.0.34",
            path_hint: PATH_HINT_SENTENCE,
          }),
      ],
    });
    renderButton({ cliAvailable: false });

    await driveToCompleted();

    // 逐字等于后端返回的整句（不截断、不加前后缀）。
    const hint = screen.getByTestId("provider-install-path-hint-codex");
    expect(hint.textContent?.trim()).toBe(PATH_HINT_SENTENCE);

    const status = screen.getByTestId("provider-install-status-codex");
    expect(status.textContent).toContain(PATH_HINT_SENTENCE);
    // 旧的二次包裹形态（「可把 … 加入 PATH」）彻底消失。
    expect(status.textContent).not.toContain("可把");
    expect(status.textContent).not.toContain(
      `可把 ${PATH_HINT_SENTENCE} 加入 PATH`,
    );
    // 版本提示不受影响。
    expect(status).toHaveTextContent("已安装：1.0.34");
  });

  it("path_hint 缺失时不渲染提示节点", async () => {
    vi.useFakeTimers();
    installFetchRouter({
      target: "codex",
      jobIds: ["job_nohint"],
      jobs: [
        () => runningJob("job_nohint", "codex"),
        () =>
          completedJobWithOutcome("job_nohint", "codex", {
            installed_version: "1.0.34",
            path_hint: null,
          }),
      ],
    });
    renderButton({ cliAvailable: false });

    await driveToCompleted();

    expect(
      screen.queryByTestId("provider-install-path-hint-codex"),
    ).toBeNull();
    const status = screen.getByTestId("provider-install-status-codex");
    expect(status).toHaveTextContent("已安装：1.0.34");
  });
});

/** §五 2.11：install / 作业轮询 / expose 三路由发送器（expose 应答可注入）。 */
function exposeFetchRouter(options: {
  target: InstallTargetWire;
  jobIds?: string[];
  jobs?: JobResponder[];
  expose?: () => Response;
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
      url === `/api/providers/${options.target}/expose`
    ) {
      return (
        options.expose?.() ??
        jsonResponse({
          exposed: true,
          command: "claude",
          dir: "/home/u/.local/bin",
          target: "/opt/node/bin/claude",
        })
      );
    }
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

// §五 2.11「把 CLI 暴露到终端 PATH」（用户明确点击才动作）：按钮仅在
// 目标已安装时出现；成功后把「若不在 PATH 上请加入 PATH」的提示**刷新**
// 为「已暴露到 <dir>，新终端可直接运行 <command>」；失败如实展示后端
// 返回的可行动原因（候选目录与各自不合格原因）。
describe("暴露到终端 PATH（§五 2.11）", () => {
  it("按钮仅在已安装时渲染（未安装不显示）", () => {
    exposeFetchRouter({ target: "claude_code" });
    const view = renderButton({
      target: "claude_code",
      cliAvailable: true,
      capabilityConfirmed: true,
    });
    expect(
      screen.getByTestId("provider-install-expose-claude_code").textContent,
    ).toBe("暴露到终端 PATH");

    view.rerender(
      <ProviderInstallActionButton
        target="claude_code"
        cliAvailable={false}
        capabilityConfirmed={true}
      />,
    );
    expect(
      screen.queryByTestId("provider-install-expose-claude_code"),
    ).toBeNull();
  });

  it("点击后 POST expose 端点；成功展示结果并刷新安装提示", async () => {
    vi.useFakeTimers();
    exposeFetchRouter({
      target: "claude_code",
      jobIds: ["job_expose"],
      jobs: [
        () => runningJob("job_expose", "claude_code"),
        () => completedJob("job_expose", "claude_code"),
      ],
    });
    const view = renderButton({ target: "claude_code", cliAvailable: false });
    await driveToCompleted();
    // 父级在 onSettled 后重读 health ⇒ 组件以「已安装」重渲。
    view.rerender(
      <ProviderInstallActionButton
        target="claude_code"
        cliAvailable={true}
        capabilityConfirmed={true}
      />,
    );
    expect(
      screen.getByTestId("provider-install-path-hint-claude_code"),
    ).toBeTruthy();

    await act(async () => {
      fireEvent.click(
        screen.getByTestId("provider-install-expose-claude_code"),
      );
      await Promise.resolve();
    });

    const result = screen.getByTestId(
      "provider-install-expose-result-claude_code",
    );
    expect(result.textContent).toContain("已暴露到 /home/u/.local/bin");
    expect(result.textContent).toContain("新终端可直接运行 claude");
    // 成功后旧的「请把 … 加入 PATH」提示被刷新掉，不再误导。
    expect(
      screen.queryByTestId("provider-install-path-hint-claude_code"),
    ).toBeNull();
  });

  it("失败时展示后端返回的可行动原因", async () => {
    exposeFetchRouter({
      target: "claude_code",
      expose: () =>
        jsonResponse(
          {
            code: "expose_no_candidate_dir",
            message:
              "没有可用的暴露目录（已安装于 /opt/node/bin/claude）：/home/u/.local/bin 真实不可写",
          },
          400,
        ),
    });
    renderButton({
      target: "claude_code",
      cliAvailable: true,
      capabilityConfirmed: true,
    });
    await act(async () => {
      fireEvent.click(
        screen.getByTestId("provider-install-expose-claude_code"),
      );
      await Promise.resolve();
    });
    const error = screen.getByTestId(
      "provider-install-expose-error-claude_code",
    );
    expect(error.textContent).toContain("expose_no_candidate_dir");
    expect(error.textContent).toContain("没有可用的暴露目录");
    expect(
      screen.queryByTestId("provider-install-expose-result-claude_code"),
    ).toBeNull();
  });

  // F3 收口（终审授权）：`exposeEnabled=false` ⇒ 本组件**不**渲染暴露按钮
  // （暴露入口改由能力卡按 off_path 口径自己渲染，避免同一动作两个同义
  // 入口）；默认（不传该 prop）⇒ 仍渲染，其它宿主（codegraph 安装入口 /
  // 注册向导）行为完全不变。
  it("exposeEnabled=false 时不渲染暴露按钮；默认仍渲染（向后兼容）", () => {
    exposeFetchRouter({ target: "claude_code" });
    const view = renderButton({
      target: "claude_code",
      cliAvailable: true,
      capabilityConfirmed: true,
      exposeEnabled: false,
    });
    expect(
      screen.queryByTestId("provider-install-expose-claude_code"),
    ).toBeNull();

    // 不传该 prop ⇒ 仍渲染暴露按钮（既有宿主行为不变）。
    view.rerender(
      <ProviderInstallActionButton
        target="claude_code"
        cliAvailable={true}
        capabilityConfirmed={true}
      />,
    );
    expect(
      screen.getByTestId("provider-install-expose-claude_code"),
    ).toBeTruthy();
  });
});
