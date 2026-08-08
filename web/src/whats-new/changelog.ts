// web/src/whats-new/changelog.ts
export interface ChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: string[];
}

export const CURRENT_VERSION = "0.0.5";

export const CHANGELOG: ChangelogEntry[] = [
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
