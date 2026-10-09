import path from "node:path";
import { realpathSync } from "node:fs";
import { spawnAriaWeb } from "../lib/aria-server.ts";
import { prepareJourneyMaterials, step } from "../lib/assemble-core.ts";
import { readProviderStatus, waitForProviderHealth } from "../lib/provider-health.ts";
import { ensureCodexTrust } from "../lib/provider-trust.ts";
import { atomicWriteJson, runDir, writeManifest } from "../lib/run-contract.ts";

/// real 全旅程装配(S0-S9 编排线):
///   1) 共享材料(lib/assemble-core):四层夹具+bare origin+空 workspace
///      +二进制新鲜度断言+装配边界基线;
///   2) codex trust 工件:~/.codex/config.toml 为本 run 的聚合根(=
///      LC provider_context_root,矩阵 ensure_provider_trust 同款唯一目标)
///      登记 trust_level="trusted";只动 HOME 既有 trust 面,幂等,CAS 拒绝覆盖;
///   3) 拉起 aria web(ARIA_PROVIDER_MODE=real,产品仅 "fake" 走桩态);
///   4) health 预热:POST /api/providers/recheck+GET status 有界轮询直到
///      codex available(矩阵 t12 先例);claude_code(五步 recipe 固定)只
///      记录事实:显式 unavailable → fail-closed,缺投影 → 如实 not_observed。
/// codex 会话 cwd=LC gateway 冻结的 canonical authority root(review_context.rs
/// cwd/target 分离合同),与 trust 登记根一致;编码 target 经 envelope 显式下发。

export const JOURNEY_STAGE_ORDER = [
  "s0",
  "s1",
  "s2",
  "s3",
  "s4",
  "s5",
  "s6",
  "s7",
  "s8",
  "s9",
] as const;

const HEALTH_TIMEOUT_MS = Number(process.env.ARIA_E2E_HEALTH_TIMEOUT_MS ?? 300_000);

async function main(): Promise<void> {
  const materials = prepareJourneyMaterials({
    runIdPrefix: "journey",
    providerMode: "real",
    stageOrder: [...JOURNEY_STAGE_ORDER],
  });
  const manifest = materials.manifest;
  manifest.providerPreflight = { codexTrust: [], health: null };

  // codex trust:canonical 聚合根(realpath 与产品 canonicalize 同口径)。
  const canonicalRoot = realpathSync(materials.aggregateRoot);
  step(`codex trust 登记根=${canonicalRoot}`);
  const trust = ensureCodexTrust(canonicalRoot);
  manifest.providerPreflight.codexTrust.push(trust);
  step(`codex trust=${trust.action}(before=${trust.beforeDigest?.slice(0, 12) ?? "-"} after=${trust.afterDigest.slice(0, 12)})`);

  const logFile = path.join(manifest.evidenceRoot, "logs", "aria-web.log");
  let server;
  try {
    server = await spawnAriaWeb({
      binary: materials.binaryPath,
      workspaceRoot: manifest.workspaceRoot,
      logFile,
      providerMode: "real",
    });
  } catch (error) {
    writeManifest(manifest);
    throw error;
  }
  manifest.baseURL = server.baseURL;
  manifest.ariaPid = server.pid;
  manifest.ariaStartTimeTicks = server.startTimeTicks;
  writeManifest(manifest);
  step(`aria web 已启动 baseURL=${server.baseURL} pid=${server.pid}(${server.startTimeTicks})`);

  // health 预热(recheck+status 有界轮询;心跳经 console 输出,同款事实进 provider-status-final.json)。
  const outcome = await waitForProviderHealth({
    baseURL: server.baseURL,
    provider: "codex",
    timeoutMs: HEALTH_TIMEOUT_MS,
    onHeartbeat: (snapshot, waitedMs) => {
      const facts = snapshot
        ? snapshot.providers.map((entry) => `${entry.provider}:${entry.available ? "ok" : "no"}${entry.version ? `@${entry.version}` : ""}`).join(",")
        : "(status 读取失败)";
      console.log(`[assemble-real] health 心跳 +${Math.round(waitedMs / 1000)}s ${facts}`);
    },
  });

  // claude_code(五步 recipe 固定 provider)只读事实核对。
  let claudeFact = "not_observed";
  try {
    const snapshot = await readProviderStatus(server.baseURL);
    const claude = snapshot.providers.find((entry) => entry.provider === "claude_code");
    if (claude) {
      claudeFact = `${claude.available ? "available" : "unavailable"}${claude.version ? `@${claude.version}` : ""}`;
    }
  } catch {
    claudeFact = "status_unreadable";
  }

  manifest.providerPreflight.health = {
    provider: "codex",
    ready: outcome.ready,
    version: outcome.version,
    waitedMs: outcome.waitedMs,
    providers: (outcome.lastSnapshot?.providers ?? []).map((entry) => ({
      provider: entry.provider,
      available: entry.available,
      version: entry.version ?? null,
    })),
  };
  if (outcome.lastSnapshot) {
    atomicWriteJson(
      path.join(manifest.evidenceRoot, "provider-status-final.json"),
      { outcome: { ...outcome, lastSnapshot: outcome.lastSnapshot }, claude: claudeFact },
    );
  }
  writeManifest(manifest);

  console.log(`[assemble-real] codex health=${outcome.ready ? "ready" : "NOT-READY"}${outcome.version ? `@${outcome.version}` : ""}(等待 ${Math.round(outcome.waitedMs / 1000)}s)`);
  console.log(`[assemble-real] claude_code(五步 recipe)=${claudeFact}`);
  console.log(`[assemble-real] 台账=${runDir(manifest.runId)}/run.json 日志=${logFile}`);

  if (!outcome.ready) {
    throw new Error(
      `codex provider health 未就绪(等待 ${Math.round(outcome.waitedMs / 1000)}s):BLOCKED——环境不可运行,证据见 provider-status-final.json;不得以 fake 桩态替代`,
    );
  }
  if (claudeFact.startsWith("unavailable")) {
    throw new Error(
      `claude_code 显式 unavailable(${claudeFact}):五步聚合初始化(固定 Claude recipe)不可运行,BLOCKED;不得换 provider 或伪造初始化`,
    );
  }

  console.log("[assemble-real] 下一步: node scripts/run-real-journey.ts(段编排 S0-S9)");
}

main().catch((error) => {
  console.error(`[assemble-real] 失败: ${(error as Error).stack ?? error}`);
  process.exit(1);
});
