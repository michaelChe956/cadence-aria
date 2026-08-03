import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

export default function (pi: ExtensionAPI) {
  pi.registerTool({
    name: "ask_user",
    label: "Ask User",
    description:
      "向用户提出一个需要决策的问题并等待回答。当需求、范围或验收标准存在必须由用户决定的歧义时使用。",
    promptSnippet: "向用户提问并等待回答（用于澄清需求歧义）",
    promptGuidelines: [
      "当需求、范围或验收标准存在必须由用户决定的歧义时，使用 ask_user 提问并等待回答，不要输出文本选择题。",
    ],
    parameters: Type.Object({
      question: Type.String({ description: "要问用户的问题" }),
      options: Type.Array(Type.String(), { description: "候选选项" }),
    }),
    async execute(_id, params, signal, _onUpdate, ctx) {
      if (signal?.aborted) {
        return { content: [{ type: "text", text: "已取消" }] };
      }
      const answer = await ctx.ui.select(params.question, params.options);
      return {
        content: [
          {
            type: "text",
            text: answer ? `用户选择：${answer}` : "用户未作选择",
          },
        ],
        details: {},
      };
    },
  });
}
