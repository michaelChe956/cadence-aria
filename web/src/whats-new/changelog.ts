export interface ChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: string[];
}

export const CURRENT_VERSION = "0.0.6";

export const CHANGELOG: ChangelogEntry[] = [
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
