import { spawn, type ChildProcess } from "node:child_process";
import { createHash } from "node:crypto";
import { closeSync, mkdirSync, openSync, readFileSync } from "node:fs";
import path from "node:path";
import { parseListeningLine, readWebEndpointFileSync } from "../env/ports.ts";
import { delay, pathExists, waitFor } from "../env/wait.ts";

/// 产品 CLI 进程管理(Rust 薄 supervisor 思想的 Node 冒烟实现):
/// 只负责拉起预构建 `aria web --port 0`、发现端点、健康等待、日志归档;
/// 不创建 project/LC/issue,不发业务 WS/POST。
///
/// 进程形态:stdio 直写日志文件(不经父进程管道)——assemble 退出后 aria
/// 继续写同一日志不受 EPIPE 影响;生命周期由台账 PID+starttime 与 teardown 收束。

export type AriaServerHandle = {
  pid: number;
  startTimeTicks: string | null;
  baseURL: string;
  port: number;
};

export function sha256File(target: string): string {
  return createHash("sha256").update(readFileSync(target)).digest("hex");
}

/** Linux /proc/<pid>/stat 的 starttime(第 22 字段),teardown 用它防 PID 复用误杀。 */
export function processStartTimeTicks(pid: number): string | null {
  try {
    const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
    const afterComm = stat.slice(stat.lastIndexOf(")") + 2).trim();
    return afterComm.split(/\s+/)[19] ?? null;
  } catch {
    return null;
  }
}

function isAlive(pid: number): boolean {
  return processStartTimeTicks(pid) !== null;
}

export type SpawnAriaOptions = {
  binary: string;
  workspaceRoot: string;
  logFile: string;
  /** 冒烟桩态:产品自带 fake provider 模式(不 spawn 任何真实 AI)。 */
  providerMode: string;
};

/** 拉起 aria web(--port 0 内核分配),从日志监听行/端点文件解析端口,等待 /api/health。 */
export async function spawnAriaWeb(options: SpawnAriaOptions): Promise<AriaServerHandle> {
  const logFile = path.resolve(options.logFile);
  mkdirSync(path.dirname(logFile), { recursive: true });
  const fd = openSync(logFile, "a");
  const child: ChildProcess = spawn(
    options.binary,
    ["web", "--workspace", options.workspaceRoot, "--host", "127.0.0.1", "--port", "0"],
    {
      stdio: ["ignore", fd, fd],
      env: {
        ...process.env,
        ARIA_PROVIDER_MODE: options.providerMode,
        NO_PROXY: "127.0.0.1,localhost",
        no_proxy: "127.0.0.1,localhost",
      },
    },
  );
  const pid = child.pid;
  if (pid === undefined) throw new Error("aria web 子进程无 PID");

  const address = await waitFor("监听行或 .aria/web-endpoint", 30_000, async () => {
    if (!isAlive(pid)) {
      const tail = await readLogTail(logFile, 4000);
      throw new Error(`aria web 提前退出,日志尾部:\n${tail}`);
    }
    const logText = (await pathExists(logFile)) ? readFileSync(logFile, "utf8") : "";
    const fromLine = parseListeningLine(logText);
    if (fromLine) return fromLine;
    try {
      return { host: "127.0.0.1", port: readWebEndpointFileSync(options.workspaceRoot) };
    } catch {
      return null;
    }
  });

  const host = address.host === "::1" ? "[::1]" : address.host;
  const baseURL = `http://${host}:${address.port}`;
  await waitFor("/api/health 200", 30_000, async () => {
    if (!isAlive(pid)) {
      const tail = await readLogTail(logFile, 4000);
      throw new Error(`aria web 健康等待期间退出,日志尾部:\n${tail}`);
    }
    try {
      const response = await fetch(`${baseURL}/api/health`, { signal: AbortSignal.timeout(2000) });
      return response.ok ? response.status : null;
    } catch {
      return null;
    }
  });

  // 关键:解除父进程对子进程的引用(libuv 的 SIGCHLD watcher 会把
  // assemble 的 node 事件循环挂住,导致 `assemble && playwright` 永不推进);
  // aria 生命周期此后完全由台账 PID+starttime 与 teardown 管理。
  child.unref();
  closeSync(fd);
  return { pid, startTimeTicks: processStartTimeTicks(pid), baseURL, port: address.port };
}

async function readLogTail(logFile: string, maxBytes: number): Promise<string> {
  if (!(await pathExists(logFile))) return "(无日志)";
  const text = readFileSync(logFile, "utf8");
  return text.slice(-maxBytes);
}

/** 停止台账登记的 aria 进程;核对 PID+starttime,只管 owned 进程。 */
export async function stopOwnedAria(
  recorded: { pid: number; startTimeTicks: string | null },
  timeoutMs = 10_000,
): Promise<string> {
  if (!isAlive(recorded.pid)) {
    return `pid ${recorded.pid} 已不存在,无需处理`;
  }
  const current = processStartTimeTicks(recorded.pid);
  if (recorded.startTimeTicks !== null && current !== recorded.startTimeTicks) {
    return `pid ${recorded.pid} starttime 不匹配(当前 ${current}),疑似 PID 复用,不动`;
  }
  try {
    process.kill(recorded.pid, "SIGTERM");
  } catch (error) {
    return `SIGTERM 失败: ${(error as Error).message}`;
  }
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline && isAlive(recorded.pid)) {
    await delay(200);
  }
  if (isAlive(recorded.pid)) {
    try {
      process.kill(recorded.pid, "SIGKILL");
    } catch {
      /* 刚好退出 */
    }
    return `SIGTERM ${timeoutMs}ms 未退出,SIGKILL 收口`;
  }
  return "SIGTERM 正常退出";
}
