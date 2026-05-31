#!/usr/bin/env node
// 本地冒烟：组当前平台子包 -> npm pack 主包+子包 -> 干净临时目录安装 -> npx 验证。
// 前置：已 `pnpm -C web build && cargo build --release`，target/release/aria 存在。

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const VERSION = "0.0.1-smoke";

function sh(cmd, args, opts = {}) {
  console.log(`$ ${cmd} ${args.join(" ")}`);
  return execFileSync(cmd, args, { stdio: "pipe", encoding: "utf8", ...opts });
}

function platformKey() {
  const arch = process.arch === "x64" ? "x64" : process.arch;
  return `${process.platform}-${arch}`;
}

function main() {
  const key = platformKey();
  const binary = path.join(repoRoot, "target/release/aria");
  if (!fs.existsSync(binary)) {
    throw new Error("target/release/aria 不存在，请先 pnpm -C web build && cargo build --release");
  }

  // 1) 组包
  sh("node", ["scripts/pack-npm.mjs", "--version", VERSION, "--main-only"], { cwd: repoRoot });
  sh("node", ["scripts/pack-npm.mjs", "--version", VERSION, "--platform", key, "--binary", binary], {
    cwd: repoRoot,
  });

  // 2) npm pack 主包 + 当前平台子包 -> tgz
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "aria-smoke-"));
  const mainTgz = sh("npm", ["pack", path.join(repoRoot, "npm/cli"), "--pack-destination", tmp], {
    cwd: repoRoot,
  })
    .trim()
    .split("\n")
    .pop();
  const subTgz = sh(
    "npm",
    ["pack", path.join(repoRoot, `npm/cli-${key}`), "--pack-destination", tmp],
    { cwd: repoRoot },
  )
    .trim()
    .split("\n")
    .pop();
  console.log(`主包 tgz: ${mainTgz}\n子包 tgz: ${subTgz}`);

  // 3) 干净项目安装两个 tgz（子包先装，主包 optionalDeps 才解析得到）。
  //    --offline：只用本地 tgz，避免 npm 去 registry 解析尚未发布的 optionalDependencies 版本而挂起。
  const proj = fs.mkdtempSync(path.join(os.tmpdir(), "aria-smoke-proj-"));
  fs.writeFileSync(path.join(proj, "package.json"), JSON.stringify({ name: "smoke", private: true }));
  sh(
    "npm",
    ["install", "--no-save", "--offline", path.join(tmp, subTgz), path.join(tmp, mainTgz)],
    { cwd: proj },
  );

  // 4) 验证执行位
  const installedBin = path.join(proj, "node_modules", "@cadence-aria", `cli-${key}`, "bin", "aria");
  const mode = fs.statSync(installedBin).mode;
  if (!(mode & 0o111)) throw new Error(`安装后 bin/aria 缺执行位：${installedBin}`);
  console.log("✓ 执行位保留");

  // 5) 验证参数透传（web --check，不起真实服务）
  const out = sh(path.join(proj, "node_modules", ".bin", "aria"), [
    "web",
    "--check",
    "--workspace",
    proj,
  ], { cwd: proj });
  if (!out.includes("web_check_ok")) throw new Error(`web --check 透传失败，输出：${out}`);
  console.log("✓ 参数透传 (web --check)");

  console.log("\n冒烟通过 ✅");
  console.log("（默认 web + 开浏览器为交互行为，请手动验证：在 " + proj + " 下运行 npx aria）");
}

main();
