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
    date: "2026-08-17",
    title: "v0.0.8 更新",
    highlights: [
      "Story/Design 对话式修订：确认页直接输入修改意见，作者增量修订并附改动摘要，循环至满意；「重新编写」推倒重来已移除，改用反馈修订表达重写意图",
      "确认双出口与可选 Review：「确认并送审」进入交叉审核，「确认定稿」直接完成；reviewer 通过不再自动定稿，报告进入对话流可继续反馈；未启用 review 的会话可凭创建快照临时送审",
      "Canvas 产物审核面板：确认阶段右侧自动滑出产物全文与「本轮改动」摘要（断线重连恢复时同样自动滑出），操作条吸顶，左侧执行节点栏保留",
      "采纳 Review 意见：一键带入完整审核报告（含 findings 明细与可选建议）到反馈框，可编辑删减后发送；产出新版本后按钮自动隐藏",
      "对照编辑：≥1440px 聚焦输入框不收起面板，可对照 artifact 撰写反馈；<1440px 自动收起避免遮挡",
      "断线恢复与存量兼容：修订中断重连后保留产物并可重试本轮修订；升级前停在旧确认阶段的会话自动迁移到新确认页",
      "视觉升级：全站蓝紫主色 + 橙强调 + 奶油底（其他工作台色系随之联动）；spec 工作台 Claymorphism 形态，新增全站设计规范 MASTER.md",
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
