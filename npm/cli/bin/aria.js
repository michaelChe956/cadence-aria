#!/usr/bin/env node
"use strict";

const { spawn } = require("node:child_process");
const http = require("node:http");
const { resolveBinary } = require("../lib/platform.js");
const { planInvocation } = require("../lib/args.js");
const { pickFreePort } = require("../lib/port.js");

// 打开浏览器（尽力而为，失败仅降级为打印地址）。
function openBrowser(url) {
  const platform = process.platform;
  const cmd = platform === "darwin" ? "open" : platform === "win32" ? "cmd" : "xdg-open";
  const args = platform === "win32" ? ["/c", "start", "", url] : [url];
  try {
    const child = spawn(cmd, args, { stdio: "ignore", detached: true });
    child.on("error", () => console.error(`无法自动打开浏览器，请手动访问：${url}`));
    child.unref();
  } catch {
    console.error(`无法自动打开浏览器，请手动访问：${url}`);
  }
}

// 轮询 /api/health 直到就绪或超时（用于决定何时开浏览器）。
function waitForReady(port, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  return new Promise((resolve) => {
    const tick = () => {
      const req = http.get(
        { host: "127.0.0.1", port, path: "/api/health", timeout: 1000 },
        (res) => {
          res.resume();
          if (res.statusCode === 200) return resolve(true);
          retry();
        },
      );
      req.on("error", retry);
      req.on("timeout", () => {
        req.destroy();
        retry();
      });
    };
    const retry = () => {
      if (Date.now() > deadline) return resolve(false);
      setTimeout(tick, 200);
    };
    tick();
  });
}

async function main() {
  const argv = process.argv.slice(2);
  const plan = planInvocation(argv);

  let binary;
  try {
    binary = resolveBinary(process.platform, process.arch, require);
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }

  let forwardArgs = plan.forwardArgs;
  let port = null;

  // 默认 web 模式：launcher 自选端口并以 web --port <p> --host 127.0.0.1 传入。
  if (plan.defaultWebMode) {
    port = await pickFreePort();
    forwardArgs = ["web", "--port", String(port), "--host", "127.0.0.1"];
  }

  const child = spawn(binary, forwardArgs, { stdio: "inherit" });

  // 信号转发，保证 Ctrl-C 干净退出。
  for (const sig of ["SIGINT", "SIGTERM"]) {
    process.on(sig, () => child.kill(sig));
  }

  // 自动开浏览器（仅默认 web 模式且未 --no-open）。
  if (plan.open && port !== null) {
    waitForReady(port).then((ready) => {
      const url = `http://127.0.0.1:${port}`;
      if (ready) openBrowser(url);
      else console.error(`服务未在预期时间内就绪，请手动访问：${url}`);
    });
  }

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
    } else {
      process.exit(code ?? 0);
    }
  });
}

main();
