#!/usr/bin/env node
// 用法：node scripts/make-release-assets.mjs --version 0.1.0 --out dist-release \
//        --binary linux-x64=path/to/aria --binary darwin-x64=... --binary darwin-arm64=...
//
// 为每个平台生成 aria-<platform>.tar.gz（含 aria + LICENSE + README.md），
// 并在 out 目录生成 SHA256SUMS。

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function parseArgs() {
  const out = { binaries: {} };
  const a = process.argv.slice(2);
  for (let i = 0; i < a.length; i++) {
    if (a[i] === "--version") out.version = a[++i];
    else if (a[i] === "--out") out.out = a[++i];
    else if (a[i] === "--binary") {
      const [key, p] = a[++i].split("=");
      out.binaries[key] = p;
    }
  }
  return out;
}

function sha256(file) {
  const buf = fs.readFileSync(file);
  return crypto.createHash("sha256").update(buf).digest("hex");
}

function main() {
  const { version, out, binaries } = parseArgs();
  if (!version || !out) throw new Error("必须指定 --version 与 --out");
  fs.mkdirSync(out, { recursive: true });

  const sums = [];
  for (const [key, binPath] of Object.entries(binaries)) {
    if (!fs.existsSync(binPath)) throw new Error(`二进制不存在：${key}=${binPath}`);
    // 组装 staging 目录：aria + LICENSE + README
    const stage = fs.mkdtempSync(path.join(os.tmpdir(), `aria-rel-${key}-`));
    fs.copyFileSync(binPath, path.join(stage, "aria"));
    fs.chmodSync(path.join(stage, "aria"), 0o755);
    fs.copyFileSync(path.join(repoRoot, "LICENSE"), path.join(stage, "LICENSE"));
    fs.copyFileSync(path.join(repoRoot, "npm/cli/README.md"), path.join(stage, "README.md"));

    const tarName = `aria-${key}.tar.gz`;
    const tarPath = path.join(out, tarName);
    // tar 保留执行位
    execFileSync("tar", ["-czf", tarPath, "-C", stage, "aria", "LICENSE", "README.md"]);
    sums.push(`${sha256(tarPath)}  ${tarName}`);
    console.log(`生成 ${tarName}`);
  }

  fs.writeFileSync(path.join(out, "SHA256SUMS"), sums.join("\n") + "\n");
  console.log(`生成 SHA256SUMS（${sums.length} 项）`);
}

main();
