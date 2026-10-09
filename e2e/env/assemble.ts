import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { spawnAriaWeb } from "../lib/aria-server.ts";
import {
  REPO_LAYERS,
  assert,
  gitCheck,
  prepareJourneyMaterials,
  step,
} from "../lib/assemble-core.ts";
import { runDir, writeManifest, type RunManifest } from "../lib/run-contract.ts";

/// 冒烟装配(fake 桩态):材料见 lib/assemble-core.ts(四层夹具仓+bare
/// origin+空 workspace+二进制新鲜度+边界基线),本入口只负责 fake provider
/// 模式下拉起服务并落台账。--dry-run 不拉服务,跑纯 Node 不变量自测。
/// 装配绝不通过 API 创建 project/LC/issue——S1 起全部动作由浏览器真实点击完成。

/** 冒烟桩态:产品自带的 fake provider 模式;不启用 ARIA_E2E_TEST_CONTROLS。 */
const PROVIDER_MODE = process.env.ARIA_E2E_PROVIDER_MODE ?? "fake";

/** --dry-run 的纯 Node 不变量自测(纪律 1:assemble/teardown 自测)。 */
function dryRunSelfCheck(manifest: RunManifest, aggregateRoot: string, originsRoot: string): void {
  step("dry-run 不变量自测开始");
  for (const layer of REPO_LAYERS) {
    const repoRoot = path.join(aggregateRoot, layer);
    assert(existsSync(path.join(repoRoot, ".git")), `${layer}: 非 git 仓`);
    const porcelain = gitCheck(repoRoot, ["status", "--porcelain"]);
    assert(porcelain.length === 0, `${layer}: 工作区不干净(${porcelain})`);
    assert(gitCheck(repoRoot, ["rev-parse", "HEAD"]).length > 0, `${layer}: 无基线提交`);
    const origin = gitCheck(repoRoot, ["config", "--get", "remote.origin.url"]);
    assert(origin === path.resolve(originsRoot, `${layer}.git`), `${layer}: origin 漂移(${origin})`);
    assert(existsSync(path.join(originsRoot, `${layer}.git`, "HEAD")), `${layer}: bare origin 缺失`);
  }
  const seedPath = path.join(aggregateRoot, "busi", "seed", "notifications.json");
  const seed = JSON.parse(readFileSync(seedPath, "utf8")) as {
    notifications: { id: string; user: string; read: boolean }[];
  };
  const entries = seed.notifications;
  assert(entries.length === 4, `seed 应 4 条,实际 ${entries.length}`);
  assert(entries.filter((n) => n.user === "e2e-user" && !n.read).length === 2, "e2e-user 未读应 2 条");
  assert(entries.filter((n) => n.user === "e2e-user" && n.read).length === 1, "e2e-user 已读应 1 条");
  assert(entries.filter((n) => n.user === "another-user" && !n.read).length === 1, "another-user 未读应 1 条");
  assert(existsSync(path.join(manifest.workspaceRoot, ".git")), "workspace: 非 git 仓");
  assert(
    gitCheck(manifest.workspaceRoot, ["log", "--oneline"]).includes("initial workspace"),
    "workspace: 基线提交缺失",
  );
  assert(manifest.ariaBinary.sha256.length === 64, "aria 二进制摘要缺失");
  step(`dry-run 自测 PASS(材料根:${runDir(manifest.runId)})`);
}

async function main(): Promise<void> {
  const dryRun = process.argv.includes("--dry-run");
  const materials = prepareJourneyMaterials({
    runIdPrefix: dryRun ? "s0s3-dry" : "s0s3",
    providerMode: PROVIDER_MODE === "fake" ? "fake(冒烟桩态,不 spawn AI)" : PROVIDER_MODE,
    stageOrder: ["s0", "s1", "s2", "s3"],
  });
  const manifest = materials.manifest;

  if (dryRun) {
    manifest.dryRun = true;
    dryRunSelfCheck(manifest, materials.aggregateRoot, materials.originsRoot);
    writeManifest(manifest);
    step(`dry-run 完成;材料保留于 ${runDir(manifest.runId)}`);
    return;
  }

  const logFile = path.join(manifest.evidenceRoot, "logs", "aria-web.log");
  let server;
  try {
    server = await spawnAriaWeb({ binary: materials.binaryPath, workspaceRoot: manifest.workspaceRoot, logFile, providerMode: PROVIDER_MODE });
  } catch (error) {
    writeManifest(manifest);
    throw error;
  }
  manifest.baseURL = server.baseURL;
  manifest.ariaPid = server.pid;
  manifest.ariaStartTimeTicks = server.startTimeTicks;
  writeManifest(manifest);

  console.log(`[assemble] baseURL=${server.baseURL} pid=${server.pid}(${server.startTimeTicks})`);
  console.log(`[assemble] 台账=${runDir(manifest.runId)}/run.json 日志=${logFile}`);
  console.log(`[assemble] 下一步: npx playwright test(或 npm run smoke 自动串装配→测试→teardown)`);
}

main().catch((error) => {
  console.error(`[assemble] 失败: ${(error as Error).stack ?? error}`);
  process.exit(1);
});
