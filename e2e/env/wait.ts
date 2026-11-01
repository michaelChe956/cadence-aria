import { access, stat } from "node:fs/promises";
import path from "node:path";

/// 有界轮询/等待工具。禁裸 sleep 推进:所有等待都是有界轮询(采样/只读)。

export async function pathExists(target: string): Promise<boolean> {
  try {
    await access(target);
    return true;
  } catch {
    return false;
  }
}

/** 有界等待文件出现(轮询采样,非忙等)。 */
export async function waitForFile(target: string, timeoutMs: number): Promise<string> {
  return waitFor("文件出现", timeoutMs, async () => ((await pathExists(target)) ? target : null));
}

/** 有界等待某条件产出非空值;超时抛错。 */
export async function waitFor<T>(
  what: string,
  timeoutMs: number,
  probe: () => Promise<T | null>,
  intervalMs = 200,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await probe();
    if (value !== null && value !== undefined) return value;
    if (Date.now() >= deadline) {
      throw new Error(`等待超时(${timeoutMs}ms): ${what}`);
    }
    await delay(Math.min(intervalMs, Math.max(1, deadline - Date.now())));
  }
}

/** 轮询采样间隔的唯一命名 seam:冒烟纪律禁裸 sleep 推进,仅采样/只读轮询可定时。 */
export function delay(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, ms);
  return promise;
}

/** 文件大小(不存在返回 null)。 */
export async function fileSize(target: string): Promise<number | null> {
  try {
    return (await stat(target)).size;
  } catch {
    return null;
  }
}

