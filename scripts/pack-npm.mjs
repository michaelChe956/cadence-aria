#!/usr/bin/env node
// 用法：
//   node scripts/pack-npm.mjs --version 0.1.0 --platform linux-x64 --binary target/release/aria
//   node scripts/pack-npm.mjs --version 0.1.0 --main-only
//
// 单平台子包组包：拷二进制 + chmod +x + 由模板写 package.json。
// 主包改写：把 npm/cli/package.json 的 version 与 optionalDependencies 版本设为目标版本。

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  if (i === -1) return fallback;
  const v = process.argv[i + 1];
  return v && !v.startsWith("--") ? v : true;
}

const SUPPORTED = ["linux-x64", "darwin-x64", "darwin-arm64"];
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function writeMainPackage(version) {
  const p = path.join(repoRoot, "npm/cli/package.json");
  const pkg = JSON.parse(fs.readFileSync(p, "utf8"));
  pkg.version = version;
  pkg.optionalDependencies = {};
  for (const key of SUPPORTED) {
    pkg.optionalDependencies[`@cadence-aria/cli-${key}`] = version; // exact
  }
  fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + "\n");
  console.log(`主包 version=${version}，optionalDependencies 已锁定 exact。`);
}

function writeSubPackage(version, key, binaryPath) {
  if (!SUPPORTED.includes(key)) throw new Error(`不支持的平台 key：${key}`);
  if (!binaryPath || !fs.existsSync(binaryPath)) {
    throw new Error(`二进制不存在：${binaryPath}`);
  }
  const subDir = path.join(repoRoot, `npm/cli-${key}`);
  const binDir = path.join(subDir, "bin");
  fs.mkdirSync(binDir, { recursive: true });

  const dest = path.join(binDir, "aria");
  fs.copyFileSync(binaryPath, dest);
  fs.chmodSync(dest, 0o755); // 关键：保执行位（设计 6.2）

  const tmpl = fs.readFileSync(path.join(subDir, "package.json.tmpl"), "utf8");
  fs.writeFileSync(
    path.join(subDir, "package.json"),
    tmpl.replace(/__VERSION__/g, version) + (tmpl.endsWith("\n") ? "" : "\n"),
  );

  // 校验执行位
  const mode = fs.statSync(dest).mode;
  if (!(mode & 0o111)) throw new Error(`bin/aria 缺少执行位：${dest}`);
  console.log(`子包 cli-${key} 组包完成，二进制可执行，version=${version}。`);
}

function main() {
  const version = arg("version");
  if (!version || version === true) throw new Error("必须指定 --version <x.y.z>");

  if (arg("main-only")) {
    writeMainPackage(version);
    return;
  }

  const platform = arg("platform");
  const binary = arg("binary");
  if (!platform || platform === true) throw new Error("必须指定 --platform <key> 或 --main-only");
  writeSubPackage(version, platform, binary);
}

main();
