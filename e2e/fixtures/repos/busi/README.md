# busi —— 通知领域层(基线骨架)

四层通知中心(`frontend → gateway → api → busi`)的领域层。本仓在基线中只包含
契约与模块位置,**不含业务实现**:成功业务响应由全旅程线的交付代码完成。

## 固定交付边界

- 领域模型:`{ id, user, title, body, created_at, read }`。
- 领域能力:按用户查询通知、未读计数、幂等标读(重复标读不改变计数)。
- 数据来源:启动时从 `NOTIFICATION_SEED_PATH` 指定的 JSON 种子载入内存;
  不承诺跨进程持久化。

## 模块位置契约

- `src/index.js`:导出带 `user` 参数的领域 API 工厂。基线为 NotImplementeds 占位,
  交付实现组合本模块后必须满足:
  - `list(user)` 只返回该用户的通知(用户隔离,不可串读);
  - `unreadCount(user)` 只统计该用户未读;
  - `markRead(user, id)` 幂等;通知不存在或不属该用户时按契约报「不存在」。

## 种子输入

`seed/notifications.json` 是固定输入数据(由 e2e/fixtures/seed.ts 生成后随基线提交):
`n1/n2` 属 `e2e-user` 未读、`n3` 属 `e2e-user` 已读、`n4` 属 `another-user` 未读。

## 技术栈

Node 原生 ESM,零运行时依赖,零 npm 安装步骤。
