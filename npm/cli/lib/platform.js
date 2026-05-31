"use strict";

// 支持的 <os>-<arch> 矩阵。新增平台时同步此处与 scripts/pack-npm.mjs 与 release workflow。
const SUPPORTED = new Set(["linux-x64", "darwin-x64", "darwin-arm64"]);

// 由 process.platform + process.arch 映射到平台子包名。
function subpackageName(platform, arch) {
  const key = `${platform}-${arch}`;
  if (!SUPPORTED.has(key)) {
    throw new Error(
      `当前平台 ${key} 暂无预编译包 (unsupported)。已支持：${[...SUPPORTED].join(", ")}。\n` +
        `可改用源码构建（需 Rust 1.95 + pnpm），或在 GitHub Release 查找对应二进制。`,
    );
  }
  return `@cadence-aria/cli-${key}`;
}

// 定位子包内的二进制路径。require.resolve 在子包未安装时抛错。
function resolveBinary(platform, arch, requireFn) {
  const pkg = subpackageName(platform, arch);
  const target = `${pkg}/bin/aria`;
  try {
    return requireFn.resolve(target);
  } catch (primaryErr) {
    // 回退：从 cwd 解析（覆盖测试桩与非常规安装布局）
    try {
      const { createRequire } = require("node:module");
      const path = require("node:path");
      const cwdRequire = createRequire(path.join(process.cwd(), "noop.js"));
      return cwdRequire.resolve(target);
    } catch {
      throw primaryErr;
    }
  }
}

module.exports = { SUPPORTED, subpackageName, resolveBinary };
