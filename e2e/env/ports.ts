import net from "node:net";
import { readFileSync } from "node:fs";
import { waitForFile } from "./wait.ts";

/// 端口与端点发现工具。
///
/// 冒烟装配采用 v2.0 §Q2 契约:`aria web --port 0` 由内核分配端口,
/// 从 stderr 监听行(`aria web listening on http://<addr>`)与 workspace 根的
/// `.aria/web-endpoint` 端口文件读取实际端口;禁止「探测到空闲再重绑」的竞态,
/// 禁止复用用户已有服务器。

/** 监听行前缀契约,与 src/web/app.rs LISTENING_LINE_PREFIX 一致。 */
export const LISTENING_LINE_PREFIX = "aria web listening on http://";

/** 从一段 stderr 文本中解析监听行(取**最后**一条:重启续跑时日志追加,
 * 旧监听行仍在文件内,首条会是已死端口的陈旧值),返回 {host, port}。 */
export function parseListeningLine(chunk: string): { host: string; port: number } | null {
  let last: { host: string; port: number } | null = null;
  for (const rawLine of chunk.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line.startsWith(LISTENING_LINE_PREFIX)) continue;
    const addr = line.slice(LISTENING_LINE_PREFIX.length).trim();
    const match = /^(?:\[(?<v6>[^\]]+)\]|(?<v4>[^:]+)):(?<port>\d+)$/.exec(addr);
    if (!match?.groups) continue;
    const port = Number.parseInt(match.groups.port, 10);
    if (!Number.isInteger(port) || port <= 0 || port > 65535) continue;
    last = { host: match.groups.v6 ?? match.groups.v4, port };
  }
  return last;
}

/** 读取 workspace 根 `.aria/web-endpoint`(内容为纯端口号)。 */
export async function readWebEndpointFile(
  workspaceRoot: string,
  timeoutMs: number,
): Promise<number> {
  const endpointFile = `${workspaceRoot}/.aria/web-endpoint`;
  const path = await waitForFile(endpointFile, timeoutMs);
  const text = readFileSync(path, "utf8").trim();
  const port = Number.parseInt(text, 10);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    throw new Error(`web-endpoint 文件内容不是合法端口: ${text}`);
  }
  return port;
}

/** 同步读取 `.aria/web-endpoint`;不存在或非法时抛错(轮询方自行捕获)。 */
export function readWebEndpointFileSync(workspaceRoot: string): number {
  const text = readFileSync(`${workspaceRoot}/.aria/web-endpoint`, "utf8").trim();
  const port = Number.parseInt(text, 10);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    throw new Error(`web-endpoint 文件内容不是合法端口: ${text}`);
  }
  return port;
}

/** 探测一个当前空闲的 TCP 端口(仅用于日志/诊断性输出,不用于重绑)。 */
export function pickFreePort(): Promise<number> {
  const { promise, resolve, reject } = Promise.withResolvers<number>();
  const server = net.createServer();
  server.unref();
  server.once("error", reject);
  server.listen(0, "127.0.0.1", () => {
    const address = server.address();
    server.close();
    if (address && typeof address === "object") {
      resolve(address.port);
    } else {
      reject(new Error("无法获取临时监听端口"));
    }
  });
  return promise;
}
