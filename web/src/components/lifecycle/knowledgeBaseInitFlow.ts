// 知识库初始化交互式两段流程的共享件（OpenSpec change
// `add-knowledge-base-interactive-init-flow`）：
//   - 输入确认区的四步操作指引文案（面板渲染，页面与卡片不另写一套）；
//   - 建库 operation 的终态等待（【确认并初始化】触发建库后轮询到终态）。
//
// 后端契约零改动：起草 / 确认记录 / 建库分叉逐字保留，本模块只做前端编排。

import { getActiveKnowledgeBaseOperation } from "../../api/knowledge-base";
import type { KnowledgeBaseOperationSnapshotDto } from "../../api/types/knowledge-base";

/**
 * 输入确认区的四步操作指引（逐字口径的唯一来源）。
 * 步骤 ③ 与后端分叉一致：全部确认后才放行 `/knowledge-base-bootstrap`。
 */
export const USER_INPUT_GUIDANCE_STEPS: readonly string[] = [
  "① 逐份打开 user-input 文件核对并补充（点文件名即在右侧编辑器打开，可直接改）",
  "② 改完点该份的「确认」；内容再改会提示需重新确认",
  "③ 全部确认后点【确认并初始化】执行知识库初始化",
  "④ 初始化完成后留在本页继续编辑知识库文档，以你的修改为准",
];

/** 建库 operation 的轮询间隔（毫秒）：与卡片侧 operation 轮询同口径。 */
export const KNOWLEDGE_BASE_OPERATION_POLL_INTERVAL_MS = 2_000;

/** operation 的终态判定：completed / failed / cancelled 三态都是终态。 */
export function isTerminalKnowledgeBaseStatus(
  status: string | null | undefined,
): boolean {
  return (
    status === "completed" || status === "failed" || status === "cancelled"
  );
}

/**
 * 等待当前 LC 的知识库 operation 进入终态。
 *
 * 以「active operation」为口径（建库期间它是本轮 operation）；返回终态快照。
 * 取数失败按「静默重试」处理（下一轮再取），直到终态或调用方放弃。
 */
export async function waitForKnowledgeBaseOperation(
  projectId: string,
  lcId: string,
  options?: { signal?: AbortSignal },
): Promise<KnowledgeBaseOperationSnapshotDto | null> {
  for (;;) {
    if (options?.signal?.aborted) {
      return null;
    }
    try {
      const operation = await getActiveKnowledgeBaseOperation(projectId, lcId);
      if (operation && isTerminalKnowledgeBaseStatus(operation.status)) {
        return operation;
      }
    } catch {
      // 轮询失败不打断建库：下一轮重试（终态以最后一次成功取数为准）。
    }
    // 等待下一轮轮询：ES2022 lib（web/tsconfig.json 的 lib 口径）没有
    // Promise.withResolvers 的类型 ⇒ 用 executor 形态。
    await new Promise<void>((resolve) => {
      const timer = window.setTimeout(resolve, KNOWLEDGE_BASE_OPERATION_POLL_INTERVAL_MS);
      options?.signal?.addEventListener(
        "abort",
        () => {
          window.clearTimeout(timer);
          resolve();
        },
        { once: true },
      );
    });
    if (options?.signal?.aborted) {
      return null;
    }
  }
}
