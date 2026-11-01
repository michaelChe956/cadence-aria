# api —— 通知 HTTP 服务层(基线骨架)

组合 `busi` 领域层,对外暴露通知 HTTP 接口。基线只含契约与模块位置,不含实现。

## 固定交付边界

- `GET /notifications?user=<user>`:该用户的通知列表(数组)。
- `GET /notifications/unread-count?user=<user>`:返回 `{ "count": <number> }`。
- `POST /notifications/{id}/read?user=<user>`:标读;返回已读通知对象(200)。
  - 标读受用户归属约束:通知不存在或不属该用户 → 404。
  - 重复标读仍 200,且未读计数不再递减(幂等)。
- 无完整认证框架;用户身份仅来自查询参数(由 gateway 层负责 X-User 映射)。

## 启动契约

- 入口:`node src/server.js`(仓库 checkout 内直接启动,零安装)。
- 环境:`BUSI_ROOT`(busi 仓 checkout 根,从中组合领域层)、
  `NOTIFICATION_SEED_PATH`(种子 JSON)、`HOST`/`PORT`(端口 0 时须打印实际监听 URL)。
- 基线占位不实现成功业务响应;交付实现不得引入新的运行时依赖。

## 技术栈

Node 原生 ESM + 原生 `node:http`,零 npm 依赖。
