import path from "node:path";
import { stopOwnedAria } from "../lib/aria-server.ts";
import { snapshotRepos } from "../lib/boundary-watch.ts";
import {
  atomicWriteJson,
  manifestPath,
  readManifest,
  readManifestPointer,
  runDir,
  stageOrderOf,
  type StageResult,
} from "../lib/run-contract.ts";

/// teardown:全停(仅台账登记且 PID+starttime 匹配的 aria 子树)+ 证据全保留。
/// 对未执行的段按 v2.0 语义回填 not_executed(台账诚实口径);
/// 段序取 manifest.stageOrder(real 全旅程 s0-s9 / 冒烟缺省 s0-s3)。
/// 不做任何全局清理(禁止 pkill 类操作);材料目录留给报告与人工检视。

function backfillNotExecuted(runId: string): void {
  const manifest = readManifest(runId);
  let sawFailure = false;
  for (const stage of stageOrderOf(manifest)) {
    const existing = manifest.stages[stage];
    if (existing && existing.status === "fail") sawFailure = true;
    if (!existing && sawFailure) {
      const result: StageResult = {
        stage,
        status: "not_executed",
        startedAt: new Date().toISOString(),
        endedAt: new Date().toISOString(),
        detail: "前置段失败,本段未执行",
      };
      manifest.stages[stage] = result;
    }
  }
  atomicWriteJson(manifestPath(runId), manifest);
}


async function main(): Promise<void> {
  const pointer = readManifestPointer();
  const root = runDir(pointer.runId);
  const manifest = readManifest(pointer.runId);

  const notes: string[] = [];
  if (pointer.dryRun) {
    notes.push("dry-run 无服务进程,仅回填台账");
  } else if (manifest.ariaPid !== null) {
    const outcome = await stopOwnedAria({
      pid: manifest.ariaPid,
      startTimeTicks: manifest.ariaStartTimeTicks,
    });
    notes.push(`aria(pid=${manifest.ariaPid}): ${outcome}`);
  } else {
    notes.push("台账无 PID,跳过进程收束");
  }

  try {
    const final = snapshotRepos({
      aggregateRoot: manifest.aggregateRoot,
      layers: ["busi", "api", "gateway", "frontend"],
      evidenceRoot: manifest.evidenceRoot,
      label: "teardown-final",
    });
    notes.push(`最终边界快照:${final.samples.length} 仓(${final.samples.map((s) => s.repo).join(",")})`);
  } catch (error) {
    notes.push(`最终边界快照失败: ${(error as Error).message}`);
  }

  backfillNotExecuted(pointer.runId);

  atomicWriteJson(path.join(root, "teardown.json"), {
    runId: pointer.runId,
    stoppedAt: new Date().toISOString(),
    notes,
    evidenceRoot: manifest.evidenceRoot,
    stages: readManifest(pointer.runId).stages,
  });
  console.log(`[teardown] ${notes.join("; ")}`);
  console.log(`[teardown] 证据保留于 ${root}(不删除)`);
}

main().catch((error) => {
  console.error(`[teardown] 失败: ${(error as Error).stack ?? error}`);
  process.exit(1);
});
