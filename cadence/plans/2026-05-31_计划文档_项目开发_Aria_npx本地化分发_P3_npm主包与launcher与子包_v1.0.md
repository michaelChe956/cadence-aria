# Aria npx 本地化分发 · P3：npm 主包与 launcher 与平台子包

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 npm 主包 `@cadence-aria/cli`（仅含 JS launcher）与三平台子包结构；launcher 完成平台映射、参数透传、空闲端口选取、默认 web 开浏览器、信号转发；组包脚本把预编译二进制塞进子包；本地 `npm pack` + `npx` 冒烟跑通。

**Architecture:** 仓库内 `npm/cli/`（主包）+ `npm/cli-<platform>/`（子包模板）。主包 `bin/aria.js` 用 `require.resolve("@cadence-aria/cli-<platform>/bin/aria")` 定位子包二进制；子包不声明 `bin`。组包脚本 `scripts/pack-npm.mjs` 把指定平台二进制拷入子包并生成 `package.json`。launcher 单测用 `node:test` + fake 二进制脚本，无需真实 aria。

**Tech Stack:** Node.js（纯 JS、`node:test`、`node:child_process`、`node:net`）、npm。

**对应设计：** v1.3 第 3、5 节，第 7.2、7.3 节；总览 P3 行。

**前置：** P1（前端嵌入）+ P2（release profile）已完成——本分册冒烟需要「发布形态」的 release 二进制。本机已确认 node v25 / pnpm 10。

---

## 文件结构

| 文件 | 职责 | 操作 |
|------|------|------|
| `npm/cli/package.json` | 主包清单（bin + optionalDependencies） | Create |
| `npm/cli/bin/aria.js` | JS launcher | Create |
| `npm/cli/README.md` | 主包说明 | Create |
| `npm/cli/lib/platform.js` | 平台映射 + 二进制定位（可单测） | Create |
| `npm/cli/lib/args.js` | 参数解析（默认 web 注入 / --no-open 剥离） | Create |
| `npm/cli/lib/port.js` | 空闲端口选取 | Create |
| `npm/cli/test/platform.test.mjs` | 平台映射单测 | Create |
| `npm/cli/test/args.test.mjs` | 参数解析单测 | Create |
| `npm/cli/test/port.test.mjs` | 空闲端口单测 | Create |
| `npm/cli/test/launch.test.mjs` | launcher 端到端（fake 二进制桩）单测 | Create |
| `npm/cli-linux-x64/package.json.tmpl` | 子包模板（占位版本） | Create |
| `npm/cli-darwin-x64/package.json.tmpl` | 子包模板 | Create |
| `npm/cli-darwin-arm64/package.json.tmpl` | 子包模板 | Create |
| `scripts/pack-npm.mjs` | 组包脚本（拷二进制 + 生成 package.json） | Create |
| `scripts/smoke-npx.mjs` | 本地 npm pack + npx 冒烟脚本 | Create |

> 平台命名统一：launcher 内部用 `<os>-<arch>`（`linux-x64`/`darwin-x64`/`darwin-arm64`），子包名 `@cadence-aria/cli-<os>-<arch>`。

---

## Task 1：平台映射模块 + 单测（TDD）

**Files:**
- Create: `npm/cli/lib/platform.js`
- Create: `npm/cli/test/platform.test.mjs`

- [ ] **Step 1: 写失败测试**

创建 `npm/cli/test/platform.test.mjs`：

```js
import { test } from "node:test";
import assert from "node:assert/strict";
import { subpackageName, SUPPORTED } from "../lib/platform.js";

test("maps linux x64 to subpackage", () => {
  assert.equal(subpackageName("linux", "x64"), "@cadence-aria/cli-linux-x64");
});

test("maps darwin x64 to subpackage", () => {
  assert.equal(subpackageName("darwin", "x64"), "@cadence-aria/cli-darwin-x64");
});

test("maps darwin arm64 to subpackage", () => {
  assert.equal(subpackageName("darwin", "arm64"), "@cadence-aria/cli-darwin-arm64");
});

test("unsupported platform throws with clear message", () => {
  assert.throws(() => subpackageName("win32", "x64"), /unsupported|不支持|win32-x64/i);
});

test("SUPPORTED lists the three target triples", () => {
  assert.deepEqual(
    [...SUPPORTED].sort(),
    ["darwin-arm64", "darwin-x64", "linux-x64"],
  );
});
```

- [ ] **Step 2: 运行测试验证失败**

Run: `node --test npm/cli/test/platform.test.mjs`
Expected: FAIL（`../lib/platform.js` 不存在）。

- [ ] **Step 3: 写实现**

创建 `npm/cli/lib/platform.js`：

```js
"use strict";

// 支持的 <os>-<arch> 矩阵。新增平台时同步此处与 scripts/pack-npm.mjs 与 release workflow。
const SUPPORTED = new Set(["linux-x64", "darwin-x64", "darwin-arm64"]);

// 由 process.platform + process.arch 映射到平台子包名。
function subpackageName(platform, arch) {
  const key = `${platform}-${arch}`;
  if (!SUPPORTED.has(key)) {
    throw new Error(
      `当前平台 ${key} 暂无预编译包。已支持：${[...SUPPORTED].join(", ")}。\n` +
        `可改用源码构建（需 Rust 1.95 + pnpm），或在 GitHub Release 查找对应二进制。`,
    );
  }
  return `@cadence-aria/cli-${key}`;
}

// 定位子包内的二进制路径。require.resolve 在子包未安装时抛错。
function resolveBinary(platform, arch, requireFn) {
  const pkg = subpackageName(platform, arch);
  return requireFn.resolve(`${pkg}/bin/aria`);
}

module.exports = { SUPPORTED, subpackageName, resolveBinary };
```

> 主包用 CommonJS（`require.resolve` 最稳）。测试用 `.mjs` + `import` 引入 CJS 默认导出的具名成员（node 支持 CJS 的命名导出互操作）。若 `import { subpackageName }` 对 CJS 失败，测试改为 `import mod from "..."; const { subpackageName } = mod;`。

- [ ] **Step 4: 运行测试验证通过**

Run: `node --test npm/cli/test/platform.test.mjs`
Expected: 5 个用例 PASS（若遇 CJS/ESM 互操作报错，按 Step 3 注解调整 import 形态后再跑）。

- [ ] **Step 5: 提交**

```bash
git add npm/cli/lib/platform.js npm/cli/test/platform.test.mjs
git commit -m "feat(npm): launcher 平台映射模块 + 单测

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2：参数解析模块 + 单测（TDD）

**Files:**
- Create: `npm/cli/lib/args.js`
- Create: `npm/cli/test/args.test.mjs`

- [ ] **Step 1: 写失败测试**

创建 `npm/cli/test/args.test.mjs`：

```js
import { test } from "node:test";
import assert from "node:assert/strict";
import { planInvocation } from "../lib/args.js";

test("no args injects default web subcommand and enables open", () => {
  const plan = planInvocation([]);
  assert.deepEqual(plan.forwardArgs, ["web"]);
  assert.equal(plan.defaultWebMode, true);
  assert.equal(plan.open, true);
});

test("explicit web subcommand without port keeps default web mode", () => {
  const plan = planInvocation(["web"]);
  assert.deepEqual(plan.forwardArgs, ["web"]);
  assert.equal(plan.defaultWebMode, true);
});

test("explicit web --port disables auto port and open", () => {
  const plan = planInvocation(["web", "--port", "3000"]);
  assert.deepEqual(plan.forwardArgs, ["web", "--port", "3000"]);
  assert.equal(plan.defaultWebMode, false);
  assert.equal(plan.open, false);
});

test("--no-open is stripped from forwarded args and disables open", () => {
  const plan = planInvocation(["--no-open"]);
  assert.deepEqual(plan.forwardArgs, ["web"]); // 无参 -> 注入 web；--no-open 被剥离
  assert.equal(plan.open, false);
  assert.equal(plan.defaultWebMode, true);
});

test("non-web subcommand forwarded verbatim, no open", () => {
  const plan = planInvocation(["task", "run", "--workspace", "/tmp/x"]);
  assert.deepEqual(plan.forwardArgs, ["task", "run", "--workspace", "/tmp/x"]);
  assert.equal(plan.defaultWebMode, false);
  assert.equal(plan.open, false);
});
```

- [ ] **Step 2: 运行测试验证失败**

Run: `node --test npm/cli/test/args.test.mjs`
Expected: FAIL（`../lib/args.js` 不存在）。

- [ ] **Step 3: 写实现**

创建 `npm/cli/lib/args.js`：

```js
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
```

- [ ] **Step 4: 运行测试验证通过**

Run: `node --test npm/cli/test/args.test.mjs`
Expected: 5 个用例 PASS。

- [ ] **Step 5: 提交**

```bash
git add npm/cli/lib/args.js npm/cli/test/args.test.mjs
git commit -m "feat(npm): launcher 参数解析(默认web注入/--no-open剥离) + 单测

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3：空闲端口选取模块 + 单测（TDD）

**Files:**
- Create: `npm/cli/lib/port.js`
- Create: `npm/cli/test/port.test.mjs`

- [ ] **Step 1: 写失败测试**

创建 `npm/cli/test/port.test.mjs`：

```js
import { test } from "node:test";
import assert from "node:assert/strict";
import { pickFreePort } from "../lib/port.js";
import net from "node:net";

test("pickFreePort returns a usable port", async () => {
  const port = await pickFreePort();
  assert.ok(Number.isInteger(port) && port > 0 && port < 65536, `端口非法: ${port}`);
  // 验证该端口当下可绑定
  await new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.once("error", reject);
    srv.listen(port, "127.0.0.1", () => srv.close(resolve));
  });
});

test("two consecutive picks both usable", async () => {
  const a = await pickFreePort();
  const b = await pickFreePort();
  assert.ok(a > 0 && b > 0);
});
```

- [ ] **Step 2: 运行测试验证失败**

Run: `node --test npm/cli/test/port.test.mjs`
Expected: FAIL（`../lib/port.js` 不存在）。

- [ ] **Step 3: 写实现**

创建 `npm/cli/lib/port.js`：

```js
"use strict";

const net = require("node:net");

// 让内核分配一个空闲端口（listen 0），取到后立即释放并返回端口号。
// 存在「释放后到二进制再 bind」的微小窗口，但本机本地场景概率极低，且二进制 bind 失败会显式报错。
function pickFreePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.once("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close((err) => (err ? reject(err) : resolve(port)));
    });
  });
}

module.exports = { pickFreePort };
```

- [ ] **Step 4: 运行测试验证通过**

Run: `node --test npm/cli/test/port.test.mjs`
Expected: 2 个用例 PASS。

- [ ] **Step 5: 提交**

```bash
git add npm/cli/lib/port.js npm/cli/test/port.test.mjs
git commit -m "feat(npm): launcher 空闲端口选取模块 + 单测

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4：launcher 主入口 bin/aria.js

**Files:**
- Create: `npm/cli/bin/aria.js`

- [ ] **Step 1: 写 launcher 主入口**

创建 `npm/cli/bin/aria.js`：

```js
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
```

- [ ] **Step 2: 语法校验**

Run: `node --check npm/cli/bin/aria.js`
Expected: 无输出（语法正确）。

- [ ] **Step 3: 提交**

```bash
git add npm/cli/bin/aria.js
git commit -m "feat(npm): launcher 主入口(定位二进制/自选端口/开浏览器/信号转发)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5：launcher 端到端单测（fake 二进制桩）

**Files:**
- Create: `npm/cli/test/launch.test.mjs`

> 用一个 fake「二进制」（实为可执行的 node/sh 脚本）替代真实 aria，验证 launcher 的端口注入、参数透传、就绪后开浏览器决策。fake 通过环境变量记录收到的参数。为避免真实开浏览器，测试设置 `--no-open` 验证「不开」路径，及默认模式下 fake 起一个 health server 验证就绪探测。

- [ ] **Step 1: 写测试**

创建 `npm/cli/test/launch.test.mjs`：

```js
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, chmodSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const launcher = fileURLToPath(new URL("../bin/aria.js", import.meta.url));

// 造一个 fake 二进制：把收到的参数写入 argsFile，若含 web --port 则起一个 health server。
function makeFakeBinary(dir, argsFile) {
  const script = `#!/usr/bin/env node
const fs = require("node:fs");
const http = require("node:http");
const args = process.argv.slice(2);
fs.writeFileSync(${JSON.stringify(argsFile)}, JSON.stringify(args));
const portIdx = args.indexOf("--port");
if (args[0] === "web" && portIdx !== -1) {
  const port = Number(args[portIdx + 1]);
  const srv = http.createServer((req, res) => {
    if (req.url === "/api/health") { res.writeHead(200); res.end('{"status":"ok"}'); }
    else { res.writeHead(404); res.end(); }
  });
  srv.listen(port, "127.0.0.1");
  // 模拟就绪行
  process.stderr.write("aria web listening on http://127.0.0.1:" + port + "\\n");
  setTimeout(() => { srv.close(); process.exit(0); }, 1500);
} else {
  process.exit(0);
}
`;
  const bin = join(dir, "aria");
  writeFileSync(bin, script);
  chmodSync(bin, 0o755);
  return bin;
}

// 在临时目录搭出 node_modules/@cadence-aria/cli-<platform>/bin/aria，让 require.resolve 命中。
function installFakeSubpackage(root, fakeBin) {
  const key = `${process.platform}-${process.arch === "x64" ? "x64" : process.arch}`;
  const pkgDir = join(root, "node_modules", "@cadence-aria", `cli-${key}`, "bin");
  require("node:fs").mkdirSync(pkgDir, { recursive: true });
  const dest = join(pkgDir, "aria");
  require("node:fs").copyFileSync(fakeBin, dest);
  chmodSync(dest, 0o755);
  // 子包 package.json（供 require.resolve 解析包根）
  writeFileSync(
    join(root, "node_modules", "@cadence-aria", `cli-${key}`, "package.json"),
    JSON.stringify({ name: `@cadence-aria/cli-${key}`, version: "0.0.0", files: ["bin"] }),
  );
  return root;
}

function runLauncher(cwd, args, extraEnv = {}) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [launcher, ...args], {
      cwd,
      env: { ...process.env, ...extraEnv },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "", stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("exit", (code) => resolve({ code, stdout, stderr }));
  });
}

test("default web mode injects --port and forwards to binary", async () => {
  const dir = mkdtempSync(join(tmpdir(), "aria-launch-"));
  const argsFile = join(dir, "args.json");
  const fakeBin = makeFakeBinary(dir, argsFile);
  installFakeSubpackage(dir, fakeBin);

  // --no-open 避免真实开浏览器
  const res = await runLauncher(dir, ["--no-open"]);
  assert.equal(res.code, 0, `launcher 退出码应为 0，stderr=${res.stderr}`);
  assert.ok(existsSync(argsFile), "fake 二进制应被调用");
  const received = JSON.parse(readFileSync(argsFile, "utf8"));
  assert.equal(received[0], "web");
  assert.ok(received.includes("--port"), "应注入 --port");
  assert.ok(received.includes("--host") && received.includes("127.0.0.1"), "应注入 --host 127.0.0.1");
});

test("explicit subcommand forwarded verbatim", async () => {
  const dir = mkdtempSync(join(tmpdir(), "aria-launch-"));
  const argsFile = join(dir, "args.json");
  const fakeBin = makeFakeBinary(dir, argsFile);
  installFakeSubpackage(dir, fakeBin);

  const res = await runLauncher(dir, ["task", "run", "--workspace", "/tmp/x"]);
  assert.equal(res.code, 0);
  const received = JSON.parse(readFileSync(argsFile, "utf8"));
  assert.deepEqual(received, ["task", "run", "--workspace", "/tmp/x"]);
});
```

> 注：`installFakeSubpackage` 依赖 launcher 的 `require.resolve` 从 **cwd 的 node_modules** 解析子包。launcher 用 `require.resolve(pkg + "/bin/aria")`，其解析基准是 launcher 文件位置而非 cwd。为让测试可控，**Step 2 将 launcher 的 resolve 改为同时尝试 cwd**（见下）。

- [ ] **Step 2: 增强 launcher 的 resolve 支持 cwd 回退（便于子包在项目 node_modules 被找到）**

实际分发中，主包与子包同装在用户项目的 `node_modules`，launcher（主包内）`require.resolve` 默认能解析同级 `@cadence-aria/*`。但为测试可控与稳健，修改 `npm/cli/lib/platform.js` 的 `resolveBinary`，在主 resolve 失败时回退到 `cwd/node_modules`：

```js
function resolveBinary(platform, arch, requireFn) {
  const pkg = subpackageName(platform, arch);
  const target = `${pkg}/bin/aria`;
  try {
    return requireFn.resolve(target);
  } catch (primaryErr) {
    // 回退：从 cwd 解析（覆盖测试桩与非常规安装布局）
    try {
      const { createRequire } = require("node:module");
      const cwdRequire = createRequire(require("node:path").join(process.cwd(), "noop.js"));
      return cwdRequire.resolve(target);
    } catch {
      throw primaryErr;
    }
  }
}
```

> 对应更新 `platform.test.mjs` 不受影响（`subpackageName` 未变）。`resolveBinary` 仍由 launcher 调用。

- [ ] **Step 3: 运行 launcher 端到端测试**

Run: `node --test npm/cli/test/launch.test.mjs`
Expected: 2 个用例 PASS。

> 若就绪/超时导致挂起，确认 fake 二进制 1.5s 后自退；测试整体应在数秒内结束。

- [ ] **Step 4: 运行 launcher 全部单测**

Run: `node --test npm/cli/test/`
Expected: platform/args/port/launch 全部用例 PASS。

- [ ] **Step 5: 提交**

```bash
git add npm/cli/lib/platform.js npm/cli/test/launch.test.mjs
git commit -m "test(npm): launcher 端到端单测(fake 二进制桩) + resolve cwd 回退

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6：主包 package.json 与 README

**Files:**
- Create: `npm/cli/package.json`
- Create: `npm/cli/README.md`

- [ ] **Step 1: 写主包 package.json**

创建 `npm/cli/package.json`（`version`/optionalDependencies 版本由发布流程注入，这里用占位 `0.0.0`，组包脚本会改写）：

```json
{
  "name": "@cadence-aria/cli",
  "version": "0.0.0",
  "description": "Aria 本地化工作台 — 通过 npx 一键启动",
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "git+https://github.com/michaelChe956/cadence-aria.git"
  },
  "type": "commonjs",
  "bin": {
    "aria": "bin/aria.js"
  },
  "files": [
    "bin/",
    "lib/",
    "README.md"
  ],
  "engines": {
    "node": ">=18"
  },
  "optionalDependencies": {
    "@cadence-aria/cli-linux-x64": "0.0.0",
    "@cadence-aria/cli-darwin-x64": "0.0.0",
    "@cadence-aria/cli-darwin-arm64": "0.0.0"
  }
}
```

- [ ] **Step 2: 写主包 README**

创建 `npm/cli/README.md`：

```markdown
# @cadence-aria/cli

通过 npx 一键在本地启动 Aria 工作台：

​```bash
npx @cadence-aria/cli
​```

无参运行会启动本地 Web 工作台（绑定 `127.0.0.1`，自动选空闲端口）并打开浏览器。

## 用法

- `npx @cadence-aria/cli`：启动 web 工作台并开浏览器。
- `npx @cadence-aria/cli --no-open`：启动 web 但不开浏览器（远程/无头环境）。
- `npx @cadence-aria/cli web --port 3000`：指定端口（不自动开浏览器）。
- `npx @cadence-aria/cli task run ...` / `daemon` / `repl`：透传给底层 aria 二进制。

## 平台支持

预编译二进制覆盖：linux-x64、darwin-x64、darwin-arm64。其它平台请使用源码构建或查阅 GitHub Release。

## 可选外部依赖

真实 provider 执行需要 `codex` / `claude` CLI 在 PATH 中；缺失不影响工作台界面与演示。
```

> README 内的代码围栏在最终文件中用三反引号；此处因嵌套用全角零宽符占位，写入文件时改回标准三反引号 ```。

- [ ] **Step 3: 校验 package.json 合法**

Run: `node -e "JSON.parse(require('node:fs').readFileSync('npm/cli/package.json','utf8')); console.log('ok')"`
Expected: 输出 `ok`。

- [ ] **Step 4: 提交**

```bash
git add npm/cli/package.json npm/cli/README.md
git commit -m "feat(npm): 主包 package.json(bin+optionalDeps) 与 README

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 7：平台子包模板

**Files:**
- Create: `npm/cli-linux-x64/package.json.tmpl`
- Create: `npm/cli-darwin-x64/package.json.tmpl`
- Create: `npm/cli-darwin-arm64/package.json.tmpl`

> 子包**不声明 `bin`**（bin 仅主包声明指向 launcher）。模板中 `__VERSION__` 由组包脚本替换。

- [ ] **Step 1: linux-x64 模板**

创建 `npm/cli-linux-x64/package.json.tmpl`：

```json
{
  "name": "@cadence-aria/cli-linux-x64",
  "version": "__VERSION__",
  "description": "Aria CLI 预编译二进制 (linux-x64)",
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "git+https://github.com/michaelChe956/cadence-aria.git"
  },
  "os": ["linux"],
  "cpu": ["x64"],
  "files": ["bin/aria"]
}
```

- [ ] **Step 2: darwin-x64 模板**

创建 `npm/cli-darwin-x64/package.json.tmpl`：

```json
{
  "name": "@cadence-aria/cli-darwin-x64",
  "version": "__VERSION__",
  "description": "Aria CLI 预编译二进制 (darwin-x64)",
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "git+https://github.com/michaelChe956/cadence-aria.git"
  },
  "os": ["darwin"],
  "cpu": ["x64"],
  "files": ["bin/aria"]
}
```

- [ ] **Step 3: darwin-arm64 模板**

创建 `npm/cli-darwin-arm64/package.json.tmpl`：

```json
{
  "name": "@cadence-aria/cli-darwin-arm64",
  "version": "__VERSION__",
  "description": "Aria CLI 预编译二进制 (darwin-arm64)",
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "git+https://github.com/michaelChe956/cadence-aria.git"
  },
  "os": ["darwin"],
  "cpu": ["arm64"],
  "files": ["bin/aria"]
}
```

- [ ] **Step 4: 校验三模板合法（替换占位后 JSON 解析）**

Run:
```bash
for f in npm/cli-linux-x64 npm/cli-darwin-x64 npm/cli-darwin-arm64; do
  sed 's/__VERSION__/0.1.0/' "$f/package.json.tmpl" | node -e "JSON.parse(require('node:fs').readFileSync(0,'utf8')); console.log('$f ok')"
done
```
Expected: 三行 `... ok`。

- [ ] **Step 5: 提交**

```bash
git add npm/cli-linux-x64 npm/cli-darwin-x64 npm/cli-darwin-arm64
git commit -m "feat(npm): 三平台子包 package.json 模板(不声明 bin)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 8：组包脚本 scripts/pack-npm.mjs

**Files:**
- Create: `scripts/pack-npm.mjs`

> 职责：给定平台 key + 版本 + 二进制路径，把二进制拷入 `npm/cli-<key>/bin/aria`、`chmod +x`、由模板生成 `package.json`；并把主包 `version` 与 optionalDependencies 版本改写为目标版本。组包脚本被本地冒烟（Task 9）与 P4 workflow 共用。

- [ ] **Step 1: 写组包脚本**

创建 `scripts/pack-npm.mjs`：

```js
#!/usr/bin/env node
"use strict";

// 用法：
//   node scripts/pack-npm.mjs --version 0.1.0 --platform linux-x64 --binary target/release/aria
//   node scripts/pack-npm.mjs --version 0.1.0 --main-only
//
// 单平台子包组包：拷二进制 + chmod +x + 由模板写 package.json。
// 主包改写：把 npm/cli/package.json 的 version 与 optionalDependencies 版本设为目标版本。

const fs = require("node:fs");
const path = require("node:path");

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  if (i === -1) return fallback;
  const v = process.argv[i + 1];
  return v && !v.startsWith("--") ? v : true;
}

const SUPPORTED = ["linux-x64", "darwin-x64", "darwin-arm64"];
const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");

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
```

- [ ] **Step 2: 校验脚本语法**

Run: `node --check scripts/pack-npm.mjs`
Expected: 无输出。

- [ ] **Step 3: 提交**

```bash
git add scripts/pack-npm.mjs
git commit -m "feat(npm): 组包脚本 pack-npm.mjs(拷二进制+chmod+x+模板渲染+主包版本)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 9：本地 npm pack + npx 冒烟脚本与验证

**Files:**
- Create: `scripts/smoke-npx.mjs`

> 本机为 linux-x64，用 P2 产出的 release 二进制组当前平台子包，`npm pack` 主包与子包，在干净临时目录安装并 `npx` 跑通。验证：二进制定位、默认 web 起服务、`--no-open`、参数透传、执行位保留。

- [ ] **Step 1: 写冒烟脚本**

创建 `scripts/smoke-npx.mjs`：

```js
#!/usr/bin/env node
"use strict";

// 本地冒烟：组当前平台子包 -> npm pack 主包+子包 -> 干净临时目录安装 -> npx 验证。
// 前置：已 `pnpm -C web build && cargo build --release`，target/release/aria 存在。

const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
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
  sh("node", ["scripts/pack-npm.mjs", "--version", VERSION, "--platform", key, "--binary", binary], { cwd: repoRoot });

  // 2) npm pack 主包 + 当前平台子包 -> tgz
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "aria-smoke-"));
  const mainTgz = sh("npm", ["pack", path.join(repoRoot, "npm/cli"), "--pack-destination", tmp], { cwd: repoRoot }).trim().split("\n").pop();
  const subTgz = sh("npm", ["pack", path.join(repoRoot, `npm/cli-${key}`), "--pack-destination", tmp], { cwd: repoRoot }).trim().split("\n").pop();
  console.log(`主包 tgz: ${mainTgz}\n子包 tgz: ${subTgz}`);

  // 3) 干净项目安装两个 tgz（子包先装，主包 optionalDeps 才解析得到）
  const proj = fs.mkdtempSync(path.join(os.tmpdir(), "aria-smoke-proj-"));
  fs.writeFileSync(path.join(proj, "package.json"), JSON.stringify({ name: "smoke", private: true }));
  sh("npm", ["install", "--no-save", path.join(tmp, subTgz), path.join(tmp, mainTgz)], { cwd: proj });

  // 4) 验证执行位
  const installedBin = path.join(proj, "node_modules", "@cadence-aria", `cli-${key}`, "bin", "aria");
  const mode = fs.statSync(installedBin).mode;
  if (!(mode & 0o111)) throw new Error(`安装后 bin/aria 缺执行位：${installedBin}`);
  console.log("✓ 执行位保留");

  // 5) 验证参数透传（web --check，不起真实服务）
  const out = sh(path.join(proj, "node_modules", ".bin", "aria"), ["web", "--check", "--workspace", proj], { cwd: proj });
  if (!out.includes("web_check_ok")) throw new Error(`web --check 透传失败，输出：${out}`);
  console.log("✓ 参数透传 (web --check)");

  console.log("\n冒烟通过 ✅");
  console.log("（默认 web + 开浏览器为交互行为，请手动验证：在 " + proj + " 下运行 npx aria）");
}

main();
```

- [ ] **Step 2: 校验脚本语法**

Run: `node --check scripts/smoke-npx.mjs`
Expected: 无输出。

- [ ] **Step 3: 跑冒烟（需已构建 release 二进制）**

Run:
```bash
pnpm -C web build && cargo build --release --locked && node scripts/smoke-npx.mjs
```
Expected: 末尾输出「冒烟通过 ✅」，含「✓ 执行位保留」「✓ 参数透传 (web --check)」。

> 若 `npm install` 因子包 `os`/`cpu` 限制在本平台之外报错：本机是 linux-x64、子包也是 linux-x64，应匹配。若 npm 对本地 tgz 的 optional 解析有差异，冒烟脚本显式同时安装主包与子包 tgz 已规避。

- [ ] **Step 4: 还原组包产生的工作区改动（避免把临时 version 提交）**

Run:
```bash
git checkout npm/cli/package.json 2>/dev/null || true
rm -rf npm/cli-*/bin npm/cli-*/package.json
git status --short
```
Expected: 工作区干净（组包产物 `bin/`、生成的 `package.json` 不入库；只有 `scripts/smoke-npx.mjs` 待提交）。

> `npm/cli-*/package.json`（无 .tmpl）与 `bin/aria` 是组包产物，应被 gitignore。下一步补 ignore。

- [ ] **Step 5: 补 .gitignore 忽略组包产物**

在仓库根 `.gitignore` 末尾追加：

```
# npm 组包产物（由 scripts/pack-npm.mjs 生成，不入库）
npm/cli-*/bin/
npm/cli-*/package.json
npm/*.tgz
```

- [ ] **Step 6: 提交**

```bash
git add scripts/smoke-npx.mjs .gitignore
git commit -m "test(npm): 本地 npm pack+npx 冒烟脚本 + 忽略组包产物

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## P3 自检（完成所有 Task 后执行）

- [ ] **launcher 全单测**：`node --test npm/cli/test/` 全绿（platform/args/port/launch）。
- [ ] **组包脚本**：`node scripts/pack-npm.mjs --version 0.1.0 --platform linux-x64 --binary target/release/aria` 成功且二进制有执行位。
- [ ] **冒烟通过**：`node scripts/smoke-npx.mjs` 输出「冒烟通过 ✅」，执行位与参数透传断言均过。
- [ ] **子包不声明 bin**：三模板均无 `bin` 字段，仅主包声明。
- [ ] **版本锁定**：组包后主包 optionalDependencies 为 exact 版本。
- [ ] **工作区干净**：组包产物已被 gitignore，未误入库。

完成后进入 **P4**。
