import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { atomicWriteJson } from "./run-contract.ts";

/// 越界看门狗:对四个成员主 checkout 做**只读** git 探测
/// (HEAD、分支、porcelain 快照),在阶段边界显式采样 + 冒烟期间周期采样。
/// 只观测不阻断;漂移证据按采样点落盘,供报告归因。

export type RepoBoundarySample = {
  repo: string;
  head: string | null;
  branch: string | null;
  porcelain: string[];
  trackedDirtyCount: number;
  untrackedCount: number;
  error?: string;
};

export type BoundarySnapshot = {
  label: string;
  at: string;
  samples: RepoBoundarySample[];
};

export type SnapshotOptions = {
  aggregateRoot: string;
  layers: string[];
  evidenceRoot: string;
  label: string;
};

function gitReadonly(cwd: string, args: string[]): string | null {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  return result.status === 0 ? (result.stdout ?? "").trim() : null;
}

function sampleRepo(aggregateRoot: string, repo: string): RepoBoundarySample {
  const repoRoot = path.join(aggregateRoot, repo);
  const head = gitReadonly(repoRoot, ["rev-parse", "HEAD"]);
  if (head === null) {
    return {
      repo,
      head: null,
      branch: null,
      porcelain: [],
      trackedDirtyCount: 0,
      untrackedCount: 0,
      error: "git 只读探测失败(仓缺失?)",
    };
  }
  const porcelain = gitReadonly(repoRoot, ["status", "--porcelain"]) ?? "<probe-failed>";
  const lines = porcelain === "" ? [] : porcelain.split(/\r?\n/).filter((line) => line.length > 0);
  const untrackedCount = lines.filter((line) => line.startsWith("??")).length;
  return {
    repo,
    head,
    branch: gitReadonly(repoRoot, ["branch", "--show-current"]),
    porcelain: lines,
    trackedDirtyCount: lines.length - untrackedCount,
    untrackedCount,
  };
}

/** 采样一轮并落盘;返回快照数据(调用方可继续断言)。 */
export function snapshotRepos(options: SnapshotOptions): BoundarySnapshot {
  const snapshot: BoundarySnapshot = {
    label: options.label,
    at: new Date().toISOString(),
    samples: options.layers.map((layer) => sampleRepo(options.aggregateRoot, layer)),
  };
  const dir = path.join(options.evidenceRoot, "boundary");
  mkdirSync(dir, { recursive: true });
  const stamp = snapshot.at.replace(/[:.]/g, "-");
  atomicWriteJson(path.join(dir, `${stamp}-${options.label}.json`), snapshot);
  return snapshot;
}

/** 阶段边界漂移判定:相对基线,HEAD 不得变、tracked 不得脏(untracked 记录待归因)。 */
export function driftSinceBaseline(
  baseline: BoundarySnapshot,
  current: BoundarySnapshot,
): string[] {
  const findings: string[] = [];
  const baseByRepo = new Map(baseline.samples.map((sample) => [sample.repo, sample]));
  for (const current_ of current.samples) {
    const base = baseByRepo.get(current_.repo);
    if (!base) {
      findings.push(`${current_.repo}: 基线无此仓`);
      continue;
    }
    if (current_.head !== base.head) findings.push(`${current_.repo}: HEAD 漂移 ${base.head} → ${current_.head}`);
    if (current_.trackedDirtyCount !== base.trackedDirtyCount) {
      findings.push(
        `${current_.repo}: tracked 脏文件数 ${base.trackedDirtyCount} → ${current_.trackedDirtyCount}(${current_.porcelain.join(" | ")})`,
      );
    }
    const added = current_.untrackedCount - base.untrackedCount;
    if (added > 0) findings.push(`${current_.repo}: 新增未跟踪文件 ${added} 个(${current_.porcelain.filter((l) => l.startsWith("??")).join(" | ")})`);
  }
  return findings;
}

/** 周期采样器(spec 存续期内运行;间隔采样属于允许的定时行为)。 */
export class BoundaryWatch {
  #timer: NodeJS.Timeout | null = null;
  readonly #options: Omit<SnapshotOptions, "label">;
  readonly #intervalMs: number;

  constructor(options: Omit<SnapshotOptions, "label">, intervalMs = 5000) {
    this.#options = options;
    this.#intervalMs = intervalMs;
  }

  start(): void {
    if (this.#timer !== null) return;
    let tick = 0;
    this.#timer = setInterval(() => {
      tick += 1;
      try {
        snapshotRepos({ ...this.#options, label: `interval-${tick}` });
      } catch {
        /* 采样失败保留已有证据,不让看门狗杀死冒烟 */
      }
    }, this.#intervalMs);
    this.#timer.unref?.();
  }

  stop(finalLabel = "watch-stop"): BoundarySnapshot | null {
    if (this.#timer === null) return null;
    clearInterval(this.#timer);
    this.#timer = null;
    return snapshotRepos({ ...this.#options, label: finalLabel });
  }
}
