import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  mkdtempSync,
  writeFileSync,
  chmodSync,
  readFileSync,
  existsSync,
  mkdirSync,
  copyFileSync,
} from "node:fs";
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
  const key = `${process.platform}-${process.arch}`;
  const pkgRoot = join(root, "node_modules", "@cadence-aria", `cli-${key}`);
  const binDir = join(pkgRoot, "bin");
  mkdirSync(binDir, { recursive: true });
  const dest = join(binDir, "aria");
  copyFileSync(fakeBin, dest);
  chmodSync(dest, 0o755);
  writeFileSync(
    join(pkgRoot, "package.json"),
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
    let stdout = "",
      stderr = "";
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
  assert.ok(
    received.includes("--host") && received.includes("127.0.0.1"),
    "应注入 --host 127.0.0.1",
  );
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
