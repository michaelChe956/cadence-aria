import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, writeFileSync, cpSync, type Stats } from "node:fs";
import { statSync, realpathSync } from "node:fs";
import path from "node:path";
import { renderSeedJson } from "../fixtures/seed.ts";
import { sha256File } from "./aria-server.ts";
import { snapshotRepos } from "./boundary-watch.ts";
import {
  E2E_ROOT,
  FIXTURES_ROOT,
  newRunId,
  runDir,
  writeManifest,
  type RunManifest,
} from "./run-contract.ts";

/// 装配核心:四层夹具仓+bare origin+空产品 workspace+run 台账+边界基线。
/// fake 冒烟线(env/assemble.ts)与 real 全旅程线(env/assemble-real.ts)共用;
/// 业务动作(project/LC/issue/会话)一律由浏览器完成,本模块零业务 API。

export const REPO_LAYERS = ["busi", "api", "gateway", "frontend"] as const;
export type RepoLayer = (typeof REPO_LAYERS)[number];

export const WORKTREE_ROOT = path.dirname(E2E_ROOT);
export const DEFAULT_BINARY = process.env.ARIA_E2E_BINARY ?? path.join(WORKTREE_ROOT, "target", "debug", "aria");

const GIT_IDENTITY = ["-c", "user.email=aria-e2e@local", "-c", "user.name=Aria Page E2E"];

export function gitRaw(cwd: string, args: string[]): { ok: boolean; stdout: string; stderr: string } {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  return { ok: result.status === 0, stdout: (result.stdout ?? "").trim(), stderr: result.stderr ?? "" };
}

export function gitCheck(cwd: string, args: string[]): string {
  const result = gitRaw(cwd, args);
  if (!result.ok) throw new Error(`git ${args.join(" ")} 失败于 ${cwd}: ${result.stderr}`);
  return result.stdout;
}

export function step(message: string): void {
  console.log(`[assemble] ${message}`);
}

export function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`装配断言失败: ${message}`);
}

export function distFingerprint(): { indexHtmlSha256: string; assetCount: number } {
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

function newestMtime(root: string): Stats {
  const self = statSync(root);
  let newest = self;
  // web/dist 的嵌入物只有文件(无子目录);仍递归以防构建形态变化。
  if (self.isDirectory()) {
    for (const entry of readdirSync(root)) {
      const candidate = newestMtime(path.join(root, entry));
      if (candidate.mtimeMs > newest.mtimeMs) newest = candidate;
    }
  }
  return newest;
}

/**
 * 二进制新鲜度断言(stale-binary 教训的装配改进项):
 * cargo build.rs 把 web/dist 嵌进 aria 二进制;dist 在二进制之后重建 →
 * 内嵌页面与磁盘 dist 不一致,spec 断言会踩旧 UI。装配时强制:
 * aria 二进制 mtime >= web/dist 全树最大 mtime,否则 fail-closed。
 */
export function assertBinaryFreshness(binary: string): void {
  const distRoot = path.join(WORKTREE_ROOT, "web", "dist");
  assert(existsSync(distRoot), `缺少 ${distRoot}:先 pnpm -C web build`);
  const newestDist = newestMtime(distRoot);
  const binaryStat = statSync(binary);
  assert(
    binaryStat.mtimeMs >= newestDist.mtimeMs,
    [
      "stale binary:aria 二进制早于 web/dist 最新产物,内嵌页面将过期。",
      `  binary mtime=${new Date(binaryStat.mtimeMs).toISOString()}`,
      `  dist   mtime=${new Date(newestDist.mtimeMs).toISOString()}(${newestDist.isFile() ? "文件" : "目录"})`,
      "  修复:pnpm -C web build && cargo build --locked --bin aria 后重跑装配。",
    ].join("\n"),
  );
  step(`二进制新鲜度 PASS(binary>=dist:${new Date(binaryStat.mtimeMs).toISOString()})`);
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

export type PreparedMaterials = {
  manifest: RunManifest;
  binaryPath: string;
  aggregateRoot: string;
  originsRoot: string;
};

/**
 * 准备一次旅程的全部材料(fake/real 共用):
 * runId+目录布局、四仓夹具+bare origin、空 workspace、二进制新鲜度、
 * aria web --check、台账落盘、装配边界基线快照。不拉起服务。
 */
export function prepareJourneyMaterials(options: {
  runIdPrefix: string;
  providerMode: string;
  stageOrder: string[];
}): PreparedMaterials {
  const runId = newRunId(options.runIdPrefix);
  const root = runDir(runId);
  const aggregateRoot = path.join(root, "aggregate-root");
  const originsRoot = path.join(root, "origins");
  const workspaceRoot = path.join(root, "workspace");
  const evidenceRoot = path.join(root, "evidence");

  step(`runId=${runId} provider=${options.providerMode} root=${root}`);
  mkdirSync(evidenceRoot, { recursive: true });

  const binary = DEFAULT_BINARY;
  assert(
    existsSync(binary),
    `aria 二进制不存在: ${binary}(先 cargo build --locked --bin aria,或设 ARIA_E2E_BINARY)`,
  );
  assertBinaryFreshness(binary);
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
    dryRun: false,
    worktreeHead,
    providerMode: options.providerMode,
    stageOrder: options.stageOrder,
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

  snapshotRepos({ aggregateRoot, layers: [...REPO_LAYERS], evidenceRoot, label: `baseline-assemble-${options.runIdPrefix}` });
  writeManifest(manifest);
  return { manifest, binaryPath: binary, aggregateRoot, originsRoot };
}

/** 成员仓绝对路径(real 线 trust/边界核对用;realpath 与产品 canonicalize 同口径)。 */
export function memberRepoRoots(aggregateRoot: string): Record<RepoLayer, string> {
  const resolved = realpathSync(aggregateRoot);
  return {
    busi: path.join(resolved, "busi"),
    api: path.join(resolved, "api"),
    gateway: path.join(resolved, "gateway"),
    frontend: path.join(resolved, "frontend"),
  };
}
