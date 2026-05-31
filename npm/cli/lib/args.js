"use strict";

// 解析用户传给 npx 的参数，决定：转发给二进制的参数、是否默认 web 模式、是否自动开浏览器。
//
// 规则（见设计 v1.3 第 5 节）：
// - 无参 -> 注入默认 `web`，默认 web 模式，自动开浏览器。
// - 仅 `web`（无 --port）-> 默认 web 模式（launcher 自选端口、开浏览器）。
// - `web --port ...` 或其它任意子命令 -> 非默认模式：尊重用户参数、不自选端口、不开浏览器。
// - `--no-open` 由 launcher 消费、从转发参数剥离，并关闭开浏览器。
function planInvocation(argv) {
  const open0 = !argv.includes("--no-open");
  const rest = argv.filter((a) => a !== "--no-open");

  if (rest.length === 0) {
    return { forwardArgs: ["web"], defaultWebMode: true, open: open0 };
  }

  const isWeb = rest[0] === "web";
  const hasPort = rest.includes("--port");
  const defaultWebMode = isWeb && !hasPort;

  return {
    forwardArgs: rest,
    defaultWebMode,
    // 仅默认 web 模式才可能开浏览器；显式 --port 或非 web 子命令一律不开。
    open: defaultWebMode && open0,
  };
}

module.exports = { planInvocation };
