import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn, spawnSync } from "node:child_process";

const workspace = mkdtempSync(join(tmpdir(), "aria-web-e2e-"));

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: workspace,
    stdio: "inherit",
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

run("git", ["init"]);
run("git", ["config", "user.email", "aria-web-e2e@example.local"]);
run("git", ["config", "user.name", "Aria Web E2E"]);
writeFileSync(join(workspace, "README.md"), "# Aria Web E2E\n");
writeFileSync(join(workspace, ".gitignore"), ".aria/\n");
run("git", ["add", "README.md", ".gitignore"]);
run("git", ["commit", "-m", "initial workspace"]);

process.env.ARIA_PROVIDER_MODE = "fake";
process.env.ARIA_E2E_TEST_CONTROLS = "1";
// 嵌入改造后，e2e 通过 ARIA_WEB_DIST 指向源码树 web/dist，
// 使前端改动无需重编 Rust（与 dev 工作流一致）。
process.env.ARIA_WEB_DIST = new URL("../dist", import.meta.url).pathname;


const child = spawn(
  "cargo",
  [
    "run",
    "--manifest-path",
    "../Cargo.toml",
    "--locked",
    "--",
    "web",
    "--workspace",
    workspace,
    "--host",
    "127.0.0.1",
    "--port",
    "4317",
  ],
  {
    cwd: new URL("..", import.meta.url),
    stdio: "inherit",
  },
);

for (const signal of ["SIGTERM", "SIGINT"]) {
  process.on(signal, () => {
    child.kill(signal);
  });
}

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 0);
});
