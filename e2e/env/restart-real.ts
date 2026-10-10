import { spawnSync } from "node:child_process";
import path from "node:path";
import { spawnAriaWeb } from "../lib/aria-server.ts";
import { waitForProviderHealth } from "../lib/provider-health.ts";
import { readManifest, readManifestPointer, runDir, writeManifest } from "../lib/run-contract.ts";

/// 为既有 run 重启 aria web(阶段化重放的前置):同一 workspace/log 追加、
/// 更新台账 pid/starttime/baseURL 与指针、health 预热复核 codex。
/// 用于服务进程死亡(或机器回收)后 `npm run journey:resume` 前的续跑装配;
/// 不创建任何新材料,不重跑装配段,不产生新 runId。

const HEALTH_TIMEOUT_MS = Number(process.env.ARIA_E2E_HEALTH_TIMEOUT_MS ?? 300_000);

async function main(): Promise<void> {
  const pointer = readManifestPointer();
  const manifest = readManifest(pointer.runId);
  if (manifest.providerMode !== "real") {
    throw new Error(`restart-real 只服务 real 台账(当前 ${manifest.providerMode})`);
  }
  // 预清本 workspace 的孤儿服务(此前失败重启可能留下持锁 aria,
  // 新服务会因 workspace 独占锁永不健康)。模式精确到本 workspace 路径,
  // 不做任何全局杀进程。
  const killPattern = `aria web --workspace ${manifest.workspaceRoot}`;
  const kill = spawnSync("pkill", ["-f", killPattern], { encoding: "utf8" });
  if (kill.status === 0) {
    console.log("[restart-real] 已清杀本 workspace 孤儿 aria(模式含完整路径)");
    await new Promise((resolve) => setTimeout(resolve, 2_000));
  }
  const logFile = path.join(manifest.evidenceRoot, "logs", "aria-web.log");
  const server = await spawnAriaWeb({
    binary: manifest.ariaBinary.path,
    workspaceRoot: manifest.workspaceRoot,
    logFile,
    providerMode: "real",
    healthTimeoutMs: 300_000,
  });
  manifest.baseURL = server.baseURL;
  manifest.ariaPid = server.pid;
  manifest.ariaStartTimeTicks = server.startTimeTicks;
  writeManifest(manifest);
  console.log(`[restart-real] aria web 重启 baseURL=${server.baseURL} pid=${server.pid}(${server.startTimeTicks})`);

  const outcome = await waitForProviderHealth({
    baseURL: server.baseURL,
    provider: "codex",
    timeoutMs: HEALTH_TIMEOUT_MS,
  });
  console.log(`[restart-real] codex health=${outcome.ready ? "ready" : "NOT-READY"}(等待 ${Math.round(outcome.waitedMs / 1000)}s)`);
  if (!outcome.ready) {
    throw new Error("codex health 未就绪:BLOCKED,不得续跑");
  }
  console.log(`[restart-real] 台账=${runDir(manifest.runId)}/run.json;下一步 npm run journey:resume`);
}

main().catch((error) => {
  console.error(`[restart-real] 失败: ${(error as Error).stack ?? error}`);
  process.exit(1);
});
