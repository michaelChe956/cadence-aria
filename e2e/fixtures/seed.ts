#!/usr/bin/env ts
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
/// 通知中心案例的固定种子数据(v2.0 §4.2 契约):
/// - n1/n2 属 e2e-user 且 read=false(首屏未读 2 条的来源)
/// - n3 属 e2e-user 且 read=true
/// - n4 属 another-user 且 read=false(用户隔离验收:不可串读)
/// 本文件只是「输入数据」的唯一事实源:assemble 把它写成 busi 仓的
/// seed/notifications.json 并随基线提交。基线阶段没有任何运行时执行它——
/// busi 的加载逻辑由全旅程线的交付实现完成。

export type NotificationSeed = {
  id: string;
  user: string;
  title: string;
  body: string;
  created_at: string;
  read: boolean;
};

export const NOTIFICATION_SEED: NotificationSeed[] = [
  {
    id: "n1",
    user: "e2e-user",
    title: "构建失败待处理",
    body: "busi 层单测在基线上未实现,本条为固定种子输入。",
    created_at: "2026-10-09T09:00:00Z",
    read: false,
  },
  {
    id: "n2",
    user: "e2e-user",
    title: "评审邀请",
    body: "请评审通知中心四层契约(输入数据,非实现)。",
    created_at: "2026-10-09T09:05:00Z",
    read: false,
  },
  {
    id: "n3",
    user: "e2e-user",
    title: "已读历史",
    body: "该条在种子中即处于已读状态。",
    created_at: "2026-10-08T18:30:00Z",
    read: true,
  },
  {
    id: "n4",
    user: "another-user",
    title: "他人通知",
    body: "属于 another-user,任何 e2e-user 请求都不得读到或标读本条。",
    created_at: "2026-10-09T10:00:00Z",
    read: false,
  },
];

export function renderSeedJson(): string {
  return `${JSON.stringify({ notifications: NOTIFICATION_SEED }, null, 2)}\n`;
}

if (process.argv[1] && process.argv[1].endsWith("seed.ts")) {
  // CLI:node fixtures/seed.ts <输出路径>
  const target = process.argv[2];
  if (!target) {
    console.error("用法: node seed.ts <notifications.json 输出路径>");
    process.exit(2);
  }
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, renderSeedJson(), "utf8");
  console.log(`seed 写入 ${target}(${NOTIFICATION_SEED.length} 条)`);
}
