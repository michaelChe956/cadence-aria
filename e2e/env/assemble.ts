import { spawnSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { renderSeedJson } from "../fixtures/seed.ts";
import { sha256File, spawnAriaWeb } from "../lib/aria-server.ts";
import { snapshotRepos } from "../lib/boundary-watch.ts";
import {
  E2E_ROOT,
  FIXTURES_ROOT,
  newRunId,
  runDir,
  writeManifest,
  type RunManifest,
} from "../lib/run-contract.ts";

/// 冒烟装配(Rust 薄 supervisor 思想的 Node 实现):
///   1) 实例化四层夹具仓(frontend/gateway/api/busi)+ 四个 bare origin
///      (ensure_member_bare_origin 同款幂等:已有 bare 不重建,origin URL 以
///      resolve 后的 bare 路径为准,仅漂移时 set-url/add);
///   2) 准备空产品 workspace(git 基线,沿 web/e2e/start-api.mjs 先例);
///   3) 拉起预构建 aria web(--port 0 内核分配端口;监听行/端点文件发现);
///   4) 写 run.json 台账 + 边界基线快照。
/// --dry-run:不拉起服务,装配材料后跑纯 Node 不变量自测(纪律 1)。
/// 装配绝不通过 API 创建 project/LC/issue——S1 起全部动作由浏览器真实点击完成。

export const REPO_LAYERS = ["busi", "api", "gateway", "frontend"] as const;
export type RepoLayer = (typeof REPO_LAYERS)[number];

const WORKTREE_ROOT = path.dirname(E2E_ROOT);
const DEFAULT_BINARY = process.env.ARIA_E2E_BINARY ?? path.join(WORKTREE_ROOT, "target", "debug", "aria");
/** 冒烟桩态:产品自带的 fake provider 模式;不启用 ARIA_E2E_TEST_CONTROLS。 */
const PROVIDER_MODE = process.env.ARIA_E2E_PROVIDER_MODE ?? "fake";
const GIT_IDENTITY = ["-c", "user.email=aria-e2e@local", "-c", "user.name=Aria Page E2E"];

function gitRaw(cwd: string, args: string[]): { ok: boolean; stdout: string; stderr: string } {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  return { ok: result.status === 0, stdout: (result.stdout ?? "").trim(), stderr: result.stderr ?? "" };
}

function gitCheck(cwd: string, args: string[]): string {
  const result = gitRaw(cwd, args);
  if (!result.ok) throw new Error(`git ${args.join(" ")} 失败于 ${cwd}: ${result.stderr}`);
  return result.stdout;
}

function step(message: string): void {
  console.log(`[assemble] ${message}`);
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`装配断言失败: ${message}`);
}

function distFingerprint(): { indexHtmlSha256: string; assetCount: number } {
  const distRoot = path.join(WORKTREE_ROOT, "web", "dist");
  const indexHtml = path.join(distRoot, "index.html");
  assert(
    existsSync(indexHtml),
    `缺少 ${indexHtml}(前端产物未构建,先 pnpm -C web build 再 cargo build)`,
  );
  const assetDir = path.join(distRoot, "assets");
  const assetCount = existsSync(assetDir) ? readdirSync(assetDir).length : 0;
  return { indexHtmlSha256: sha256File(indexHtml), assetCount };
}

/** 实例化一个成员仓:复制模板→写 seed(仅 busi)→git init+基线提交。 */
function instantiateMemberRepo(layer: RepoLayer, aggregateRoot: string): void {
  const repoRoot = path.join(aggregateRoot, layer);
  cpSync(path.join(FIXTURES_ROOT, "repos", layer), repoRoot, { recursive: true });
  if (layer === "busi") {
    mkdirSync(path.join(repoRoot, "seed"), { recursive: true });
    writeFileSync(path.join(repoRoot, "seed", "notifications.json"), renderSeedJson(), "utf8");
  }
  gitCheck(repoRoot, ["init", "-q", "-b", "main"]);
  gitCheck(repoRoot, [...GIT_IDENTITY, "add", "-A"]);
  gitCheck(repoRoot, [...GIT_IDENTITY, "commit", "-q", "-m", `baseline: ${layer} 层契约骨架`]);
}

/** ensure_member_bare_origin 同款幂等:bare 不存在才建;origin URL 只在漂移时纠正。 */
function ensureBareOrigin(originsRoot: string, aggregateRoot: string, layer: RepoLayer): string {
  const bare = path.join(originsRoot, `${layer}.git`);
  if (!existsSync(path.join(bare, "HEAD"))) {
    mkdirSync(bare, { recursive: true });
    gitCheck(bare, ["init", "-q", "--bare", "-b", "main"]);
  }
  const desired = path.resolve(bare);
  const repoRoot = path.join(aggregateRoot, layer);
  const current = gitRaw(repoRoot, ["config", "--get", "remote.origin.url"]).stdout;
  if (current === desired) return bare;
  if (current.length > 0) {
    gitCheck(repoRoot, ["remote", "set-url", "origin", desired]);
  } else {
    gitCheck(repoRoot, ["remote", "add", "origin", desired]);
  }
  return bare;
}

function prepareWorkspace(workspaceRoot: string): void {
  mkdirSync(workspaceRoot, { recursive: true });
  gitCheck(workspaceRoot, ["init", "-q", "-b", "main"]);
  writeFileSync(path.join(workspaceRoot, "README.md"), "# Aria Page E2E workspace\n");
  writeFileSync(path.join(workspaceRoot, ".gitignore"), ".aria/\n");
  gitCheck(workspaceRoot, [...GIT_IDENTITY, "add", "README.md", ".gitignore"]);
  gitCheck(workspaceRoot, [...GIT_IDENTITY, "commit", "-q", "-m", "initial workspace"]);
}

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
  const runId = newRunId(dryRun ? "s0s3-dry" : "s0s3");
  const root = runDir(runId);
  const aggregateRoot = path.join(root, "aggregate-root");
  const originsRoot = path.join(root, "origins");
  const workspaceRoot = path.join(root, "workspace");
  const evidenceRoot = path.join(root, "evidence");

  step(`runId=${runId} dryRun=${dryRun} root=${root}`);
  mkdirSync(evidenceRoot, { recursive: true });

  const binary = DEFAULT_BINARY;
  assert(
    existsSync(binary),
    `aria 二进制不存在: ${binary}(先 cargo build --locked --bin aria,或设 ARIA_E2E_BINARY)`,
  );
  const check = spawnSync(binary, ["web", "--check", "--workspace", workspaceRoot], { encoding: "utf8" });
  assert(check.status === 0, `aria web --check 失败: ${check.stderr ?? ""}`);
  const worktreeHead = gitCheck(WORKTREE_ROOT, ["rev-parse", "HEAD"]);

  for (const layer of REPO_LAYERS) {
    instantiateMemberRepo(layer, aggregateRoot);
    ensureBareOrigin(originsRoot, aggregateRoot, layer);
  }
  prepareWorkspace(workspaceRoot);

  const manifest: RunManifest = {
    runId,
    createdAt: new Date().toISOString(),
    dryRun,
    worktreeHead,
    providerMode: PROVIDER_MODE === "fake" ? "fake(冒烟桩态,不 spawn AI)" : PROVIDER_MODE,
    ariaBinary: { path: path.resolve(binary), sha256: sha256File(binary) },
    distFingerprint: distFingerprint(),
    baseURL: null,
    workspaceRoot,
    aggregateRoot,
    originsRoot,
    evidenceRoot,
    ariaPid: null,
    ariaStartTimeTicks: null,
    observed: {},
    stages: {},
  };

  snapshotRepos({ aggregateRoot, layers: [...REPO_LAYERS], evidenceRoot, label: "baseline-assemble" });

  if (dryRun) {
    dryRunSelfCheck(manifest, aggregateRoot, originsRoot);
    writeManifest(manifest);
    step(`dry-run 完成;材料保留于 ${root}`);
    return;
  }

  const logFile = path.join(evidenceRoot, "logs", "aria-web.log");
  let server;
  try {
    server = await spawnAriaWeb({ binary, workspaceRoot, logFile, providerMode: PROVIDER_MODE });
  } catch (error) {
    writeManifest(manifest);
    throw error;
  }
  manifest.baseURL = server.baseURL;
  manifest.ariaPid = server.pid;
  manifest.ariaStartTimeTicks = server.startTimeTicks;
  writeManifest(manifest);

  console.log(`[assemble] baseURL=${server.baseURL} pid=${server.pid}(${server.startTimeTicks})`);
  console.log(`[assemble] 台账=${path.join(root, "run.json")} 日志=${logFile}`);
  console.log(`[assemble] 下一步: npx playwright test(或 npm run smoke 自动串装配→测试→teardown)`);
}

main().catch((error) => {
  console.error(`[assemble] 失败: ${(error as Error).stack ?? error}`);
  process.exit(1);
});
