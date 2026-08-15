export interface ChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: string[];
}

export const CURRENT_VERSION = "0.0.8";

export const CHANGELOG: ChangelogEntry[] = [
  {
    version: "0.0.8",
    date: "2026-08-15",
    title: "v0.0.8 更新",
    highlights: [
      "Story/Design 工作台支持对话式修订：确认页直接输入修改意见，作者增量修订并附改动摘要",
      "确认双出口：「确认并送审」进入交叉审核，「确认定稿」直接完成；审核结论不再自动定稿",
      "审核报告进入对话流：pass 或返修统一回到确认页，可基于报告继续反馈修订",
      "临时送审与断线恢复增强：未启用 review 的会话可一键送审，修订中断重连后可继续",
    ],
  },
  {
    version: "0.0.7",
    date: "2026-08-13",
    title: "v0.0.7 更新",
    highlights: [
      "新增 Kimi Code Provider：工作台可选 Kimi Code 作为作者/审核者，支持提问与 Supervised 审批",
      "图片创建支持选择 Kimi Code 提供方",
      "Kimi 会话稳定性修复：断线恢复、会话续接、超时跟随工作区总超时",
      "修复 Work Item Plan 生成链路：outline 校验契约、设计上下文识别、阻塞状态操作引导",
    ],
  },
  {
    version: "0.0.6",
    date: "2026-08-08",
    title: "v0.0.6 更新",
    highlights: [
      "新增版本更新弹窗：每次发版后打开工作台会展示本次更新内容",
      "Coder 提交与 Provider 自动重试，提升执行稳定性",
      "新增人工组级最终确认：Work Item 完成后由用户手动确认组级完成",
    ],
  },
  {
    version: "0.0.5",
    date: "2026-08-08",
    title: "v0.0.5 更新",
    highlights: [
      "Coder 提交与 Provider 重试",
      "新增人工组级最终确认",
      "移除图片客户端临时诊断日志",
    ],
  },
];
