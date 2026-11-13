#!/usr/bin/env node
// codegraph 聚合索引成员覆盖验收 helper（只读，不做任何写操作）。
//
// 用法：
//   node codegraph_coverage_helper.mjs --sdk <npm-sdk.js 绝对路径> \
//     --root <聚合根> --members '["member-a","member-b"]'
//
// stdout 只输出一个 JSON：
//   成功：{"ok":true,"members":{"<name>":true|false},"stats":{…},
//          "index":{"state":"…","pending":N,"stale":bool}}
//   失败：{"ok":false,"code":"…","message":"…"} 并以非 0 退出
// （栈与诊断只走 stderr，不污染 stdout 协议）。
//
// SDK 是全局安装的 npm 包（CJS 入口 npm-sdk.js），不能被普通脚本直接
// import ⇒ 按绝对路径 import 并做 CJS interop（exports 里只有
// default/module.exports，没有具名导出）：`mod.default ?? mod` 取 CodeGraph。

import { pathToFileURL } from "node:url";

function emit(value) {
  const payload = JSON.stringify(value) + "\n";
  // 兜底退出：stdout 回调异常（如 EPIPE）时也要离开，不悬挂父进程超时。
  const timer = setTimeout(() => process.exit(value.ok ? 0 : 1), 2000);
  timer.unref();
  process.stdout.write(payload, () => {
    clearTimeout(timer);
    process.exit(value.ok ? 0 : 1);
  });
}

function fail(code, message) {
  emit({ ok: false, code, message });
}

function argValue(name) {
  const argv = process.argv.slice(2);
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : undefined;
}

// 健康字段尽力而为：取不到就省略该字段，不因健康探针失败报错。
async function attempt(fn) {
  try {
    return { value: await fn() };
  } catch (error) {
    process.stderr.write(`[coverage-helper] probe failed: ${String(error?.message ?? error)}\n`);
    return { error };
  }
}

async function main() {
  const sdkPath = argValue("--sdk");
  const root = argValue("--root");
  const membersRaw = argValue("--members");
  if (!sdkPath || !root || !membersRaw) {
    return fail("bad_arguments", "missing --sdk/--root/--members argument");
  }
  let members;
  try {
    members = JSON.parse(membersRaw);
  } catch {
    return fail("bad_arguments", "--members is not valid JSON");
  }
  if (!Array.isArray(members) || members.some((member) => typeof member !== "string")) {
    return fail("bad_arguments", "--members must be a JSON array of strings");
  }

  const mod = await import(pathToFileURL(sdkPath).href);
  const pkg = mod.default ?? mod;
  const CodeGraph = pkg?.CodeGraph ?? mod?.CodeGraph;
  if (typeof CodeGraph !== "function" || typeof CodeGraph.open !== "function") {
    return fail("sdk_incompatible", `${sdkPath} does not export CodeGraph.open`);
  }
  // 显式只读：上游默认 open 可能迁移 schema / 自愈索引 / 维护 WAL，不是
  // 严格只读。1.6.2 的 `CodeGraph.open(projectRoot, { readOnly: true })` 支持
  // 该选项（`src/index.ts:120 OpenOptions.readOnly`、`src/index.ts:388`）。
  // 注意：上游自家 `status` 命令**并非**只读打开（1.6.2 实测
  // `src/bin/codegraph.ts:1094` 是 `CodeGraph.open(projectPath)`，没有传
  // readOnly，已初始化索引上走读写 + WAL 自愈），故本 helper 的只读口径
  // 是自己的选择，不能拿 `status` 当旁证。
  const cg = await CodeGraph.open(root, { readOnly: true });
  if (typeof cg.hasFilesUnder !== "function") {
    return fail("sdk_incompatible", "CodeGraph instance has no hasFilesUnder()");
  }

  const covered = {};
  for (const member of members) {
    covered[member] = Boolean(await cg.hasFilesUnder(member));
  }

  const output = { ok: true, members: covered };
  const stats = await attempt(() => cg.getStats());
  if (!stats.error) {
    output.stats = stats.value;
  }
  const index = {};
  const state = await attempt(async () => {
    const value = await cg.getIndexState?.();
    return value && typeof value === "object" && "state" in value
      ? value.state
      : value;
  });
  if (!state.error && state.value !== undefined) {
    index.state = state.value;
  }
  const pending = await attempt(async () => {
    if (typeof cg.getPendingFiles !== "function") {
      return undefined;
    }
    const files = await cg.getPendingFiles();
    return (files ?? []).length;
  });
  if (!pending.error && pending.value !== undefined) {
    index.pending = pending.value;
  }
  const stale = await attempt(async () => await cg.isIndexStale?.());
  if (!stale.error && stale.value !== undefined) {
    index.stale = Boolean(stale.value);
  }
  if (Object.keys(index).length > 0) {
    output.index = index;
  }

  // 只读使用后归还句柄；关闭失败不影响已得结果。
  try {
    await cg.close?.();
  } catch (error) {
    process.stderr.write(`[coverage-helper] close failed: ${String(error?.message ?? error)}\n`);
  }
  emit(output);
}

main().catch((error) => {
  fail("helper_failed", String(error?.message ?? error));
});
