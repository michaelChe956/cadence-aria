export interface ChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: string[];
}

export const CURRENT_VERSION = "0.1.11";

// 发布升版本时请裁剪数组，仅保留最新 4 条。
export const CHANGELOG: ChangelogEntry[] = [
  {
    version: "0.1.11",
    date: "2026-11-20",
    title: "日志包保留聚合初始化的审计拒绝原因",
    highlights: [
      "聚合初始化被文件审计拒绝时，失败记录会列出触发拒绝的步骤、变更分类和相对路径样本，即使完整审计回执太大无法打包也能排查。",
      "摘要不含文件内容或符号链接目标，不改变现有审计规则。",
    ],
  },
  {
    version: "0.1.10",
    date: "2026-11-20",
    title: "工作台先显示议题列表 · 切换与删除不再错乱",
    highlights: [
      "刷新工作台时议题标题先出现：选中的议题优先读取生命周期，其余议题的阶段状态分批更新。40 条议题时，列表出现时间从 346 ms 降到 204 ms。",
      "切换项目不再串数据：旧项目的操作完成后不会把页面切走，两个项目里同编号的议题也不会显示成对方的内容。",
      "删除即删除：产物删除成功后旧卡片立即消失，重新读取失败也不会放回来；删除中的议题不会重复发送删除请求。",
      "失败与加载都看得见：链接指向的产物读不出来时页面直接说明原因；议题列表还在加载时不再误报「没有匹配的 Issue」。",
    ],
  },
  {
    version: "0.1.9",
    date: "2026-11-19",
    title: "重建聚合索引：查询耗时不再挤占源码复核时间",
    highlights: [
      "CodeGraph 返回覆盖名单后，Aria 才开始计算盘检的 2 秒时限。只有确认无源码的成员才跳过；无法完成检查时仍会提示原因。",
      "重建失败后，卡片会重新读取本次索引状态，不再停留在上一次的诊断。",
      "日志包可查看本次重建的覆盖数、分段耗时和逐仓盘检结果。Pi 会话识别支持更多原生存放方式，宿主批准命令默认使用 npx。",
    ],
  },
  {
    version: "0.1.8",
    date: "2026-11-19",
    title: "配置仓不再拖垮整库索引 · 生成 Story 前自动更新索引",
    highlights: [
      "只有配置和文档、没有源码的成员仓不再让聚合索引构建失败：确认无源码即跳过并点名提示，该仓仍是项目成员、仍可作为 Issue 目标仓",
      "生成或重新生成 Story 前自动确认共享索引与本地代码一致：有变化或无法确认未变（工作区有未提交修改）时先增量同步并重新验收，失败停在 AI 启动之前并给出稳定码",
      "纯配置仓作为 Issue 目标时不再发起 CodeGraph 证据查询，provider 直接在该仓工作区读写配置文件",
      "聚合索引失败提示改为「问题 / 原因 / 怎么做 + 机器码」三段并点名成员，冷启动卡片、HTTP 错误与只读诊断三处同码同文案",
    ],
  },
];

export function recentEntries(currentVersion: string, limit = 4): ChangelogEntry[] {
  const currentVersionIndex = CHANGELOG.findIndex((item) => item.version === currentVersion);
  if (currentVersionIndex === -1) {
    return [];
  }

  return CHANGELOG.slice(currentVersionIndex, currentVersionIndex + limit);
}
