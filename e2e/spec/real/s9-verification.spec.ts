import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { mkdirSync, openSync, readFileSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { readManifest } from "../../lib/run-contract.ts";
import { delay } from "../../env/wait.ts";
import { FOUR_LAYERS, bootstrapRealJourney, readIssueLifecycle, requirePreviousCleared, runStage, screenshotOf } from "../../lib/journey-support.ts";

/// real 全旅程 S9:交付代码纵切验收(从远端交付快照运行,四层真链路)。
/// - 逐仓:git ls-remote 只读留证 → 从 bare origin 克隆交付分支到 /tmp
///   (/tmp 强制规则;origin 只读);
/// - 启动 api(BUSI_ROOT/NOTIFICATION_SEED_PATH/PORT=0)与 gateway
///   (API_BASE_URL/FRONTEND_ROOT/PORT=0),解析监听 URL;
/// - 浏览器:经 gateway 同源打开通知中心(基线契约 data-testid=
///   notification-center):首屏未读 2 → 点击未读变 1 → 重复点击仍 1;
/// - Node API 断言(X-User 真实头):幂等标读、缺失 401、another-user
///   隔离(不可串读/不可代标)、X-User 优先于 query user。

const STAGE_TEST_TIMEOUT = 600_000;
const bootstrap = bootstrapRealJourney();

function gitCheck(cwd: string, args: string[]): string {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git ${args.join(" ")} 失败:${result.stderr}`);
  return (result.stdout ?? "").trim();
}

async function readLogTail(logFile: string): Promise<string> {
  try {
    return readFileSync(logFile, "utf8");
  } catch {
    return "";
  }
}

/** 端口 0 启动交付层子进程,从 stdout 解析首个 http 监听 URL(README 契约)。 */
async function spawnLayer(options: {
  name: string;
  cwd: string;
  env: Record<string, string>;
  logFile: string;
  timeoutMs: number;
}): Promise<{ child: ChildProcess; url: string }> {
  mkdirSync(path.dirname(options.logFile), { recursive: true });
  const fd = openSync(options.logFile, "a");
  const child = spawn(process.execPath, ["src/server.js"], {
    cwd: options.cwd,
    env: { ...process.env, ...options.env },
    stdio: ["ignore", fd, fd],
  });
  const deadline = Date.now() + options.timeoutMs;
  for (;;) {
    const log = await readLogTail(options.logFile);
    const match = /http:\/\/[^\s]+/.exec(log);
    if (match) return { child, url: match[0] };
    if (child.exitCode !== null) {
      throw new Error(`${options.name} 提前退出(code=${child.exitCode}),日志:${log}`);
    }
    if (Date.now() >= deadline) {
      throw new Error(`${options.name} 未在 ${options.timeoutMs}ms 内打印监听 URL,日志:${log}`);
    }
    await delay(500);
  }
}

async function apiCall(baseURL: string, pathname: string, init?: RequestInit): Promise<{ status: number; body: unknown }> {
  const response = await fetch(`${baseURL}${pathname}`, { ...init, signal: AbortSignal.timeout(10_000) });
  const text = await response.text();
  let body: unknown = null;
  try {
    body = text.length > 0 ? JSON.parse(text) : null;
  } catch {
    body = { raw: text.slice(0, 500) };
  }
  return { status: response.status, body };
}

/** 未读计数运行时收窄(边界为交付代码的 HTTP 响应)。 */
function countOf(body: unknown, label: string): number {
  if (body !== null && typeof body === "object" && "count" in body && typeof (body as { count: unknown }).count === "number") {
    return (body as { count: number }).count;
  }
  throw new Error(`${label} 响应缺 count 数字字段:${JSON.stringify(body)}`);
}

/** 通知列表运行时收窄:id 数组(用户隔离断言用)。 */
function idsOf(body: unknown, label: string): string[] {
  if (Array.isArray(body)) {
    return body.map((entry) => {
      if (entry !== null && typeof entry === "object" && "id" in entry && typeof (entry as { id: unknown }).id === "string") {
        return (entry as { id: string }).id;
      }
      throw new Error(`${label} 列表元素缺 id:${JSON.stringify(entry)}`);
    });
  }
  throw new Error(`${label} 响应非数组:${JSON.stringify(body)}`);
}

/** 页面内未读徽标数字(semantic:通知中心容器内的计数文本)。 */
async function unreadBadge(page: Page): Promise<string> {
  const center = page.getByTestId("notification-center");
  await expect(center).toBeVisible();
  const badge = center.getByTestId("unread-badge");
  if ((await badge.count()) > 0) return (await badge.innerText()).trim();
  // 无专用 testid 时取容器内计数语义文本(「未读 2」/「2 条未读」)。
  const text = await center.innerText();
  const match = /未读[^0-9]{0,4}([0-9]+)|([0-9]+)[^0-9]{0,3}未读/.exec(text);
  return match?.[1] ?? match?.[2] ?? "";
}

test.describe("real 全旅程 S9 通知中心纵切", () => {
  test.setTimeout(STAGE_TEST_TIMEOUT);

  test("S9 远端交付快照运行与业务验收", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s9");
    const runtimes: { name: string; child: ChildProcess }[] = [];
    try {
      await runStage(bootstrap, "s9", async () => {
        const manifest = readManifest(bootstrap.runId);
        const lifecycle = await readIssueLifecycle(bootstrap);
        const summary = lifecycle.delivery_summary;
        expect(summary?.overall).toBe("all_pushed");

        // 1) 远端只读留证 + 克隆交付分支(/tmp 强制规则)。
        const runtimeRoot = path.join(os.tmpdir(), `aria-e2e-s9-${bootstrap.runId}`);
        const evidenceDir = path.join(bootstrap.evidenceRoot, "s9-runtime");
        mkdirSync(evidenceDir, { recursive: true });
        const clones: Record<string, string> = {};
        for (const layer of FOUR_LAYERS) {
          const origin = path.join(manifest.originsRoot, `${layer}.git`);
          const refs = gitCheck(os.tmpdir(), ["ls-remote", origin]);
          writeFileSync(path.join(evidenceDir, `ls-remote-${layer}.txt`), refs, "utf8");
          const entry = summary!.entries.find((candidate) => candidate.repository_name.includes(layer));
          expect(entry, `delivery 缺 ${layer} 条目`).toBeTruthy();
          const cloneTarget = path.join(runtimeRoot, layer);
          gitCheck(os.tmpdir(), ["clone", "--quiet", "--branch", entry!.branch_name!, origin, cloneTarget]);
          clones[layer] = cloneTarget;
          const head = gitCheck(cloneTarget, ["rev-parse", "HEAD"]);
          expect(head, `${layer} 克隆 HEAD 与交付 SHA 不一致`).toBe(entry!.commit_sha);
        }

        // 2) 启动交付栈(api+gateway;busi 经 BUSI_ROOT 组合;frontend 经 FRONTEND_ROOT)。
        const apiHandle = await spawnLayer({
          name: "api",
          cwd: clones.api!,
          env: {
            BUSI_ROOT: clones.busi!,
            NOTIFICATION_SEED_PATH: path.join(clones.busi!, "seed", "notifications.json"),
            HOST: "127.0.0.1",
            PORT: "0",
          },
          logFile: path.join(evidenceDir, "api.log"),
          timeoutMs: 60_000,
        });
        runtimes.push({ name: "api", child: apiHandle.child });
        const gatewayHandle = await spawnLayer({
          name: "gateway",
          cwd: clones.gateway!,
          env: {
            API_BASE_URL: apiHandle.url,
            FRONTEND_ROOT: clones.frontend!,
            HOST: "127.0.0.1",
            PORT: "0",
          },
          logFile: path.join(evidenceDir, "gateway.log"),
          timeoutMs: 60_000,
        });
        runtimes.push({ name: "gateway", child: gatewayHandle.child });
        const gatewayURL = gatewayHandle.url;

        // 3) 浏览器经 gateway 同源验收(先 UI 后 API,标读顺序:n1 由 UI)。
        await page.goto(gatewayURL);
        await expect(page.getByTestId("notification-center")).toBeVisible({ timeout: 30_000 });
        expect(await unreadBadge(page), "首屏未读计数应 2").toBe("2");
        await screenshotOf(bootstrap, page, "s9-first-screen");

        const item = page.getByTestId("notification-center").getByText("构建失败待处理", { exact: false });
        await expect(item).toBeVisible();
        await item.click();
        await expect
          .poll(async () => unreadBadge(page), { timeout: 30_000 })
          .toBe("1");
        await item.click();
        await expect
          .poll(async () => unreadBadge(page), { timeout: 15_000 })
          .toBe("1");
        await screenshotOf(bootstrap, page, "s9-after-mark-read");

        // 4) API 面断言(X-User 真实头,经 gateway 同源)。
        const user = { "X-User": "e2e-user" };
        const other = { "X-User": "another-user" };
        const assertions: Record<string, unknown> = {};

        const countAfterUi = await apiCall(gatewayURL, "/api/notifications/unread-count?user=e2e-user", { headers: user });
        assertions.countAfterUi = countAfterUi;
        expect(countAfterUi.status).toBe(200);
        expect(countOf(countAfterUi.body, "UI 标读后计数")).toBe(1);

        const markN2 = await apiCall(gatewayURL, "/api/notifications/n2/read?user=e2e-user", { method: "POST", headers: user });
        assertions.markN2 = markN2;
        expect(markN2.status).toBe(200);
        const repeatN2 = await apiCall(gatewayURL, "/api/notifications/n2/read?user=e2e-user", { method: "POST", headers: user });
        assertions.repeatN2 = repeatN2;
        expect(repeatN2.status).toBe(200);
        const countZero = await apiCall(gatewayURL, "/api/notifications/unread-count?user=e2e-user", { headers: user });
        assertions.countZero = countZero;
        expect(countOf(countZero.body, "幂等标读后计数")).toBe(0);

        const noUser = await apiCall(gatewayURL, "/api/notifications/unread-count?user=e2e-user");
        assertions.noUser = noUser.status;
        expect(noUser.status).toBe(401);

        const otherList = await apiCall(gatewayURL, "/api/notifications?user=another-user", { headers: other });
        assertions.otherList = otherList;
        expect(otherList.status).toBe(200);
        expect(idsOf(otherList.body, "another-user 列表")).toEqual(["n4"]);

        const crossMark = await apiCall(gatewayURL, "/api/notifications/n1/read?user=another-user", { method: "POST", headers: other });
        assertions.crossMark = crossMark.status;
        expect(crossMark.status).toBe(404);

        const override = await apiCall(gatewayURL, "/api/notifications?user=another-user", { headers: user });
        assertions.override = override;
        expect(override.status).toBe(200);
        expect(idsOf(override.body, "X-User 优先列表").sort()).toEqual(["n1", "n2", "n3"]);

        writeFileSync(path.join(evidenceDir, "api-assertions.json"), JSON.stringify(assertions, null, 2), "utf8");
        await screenshotOf(bootstrap, page, "s9-final");
      });
    } finally {
      for (const runtime of runtimes) {
        if (runtime.child.exitCode === null) runtime.child.kill("SIGTERM");
      }
    }
  });
});
