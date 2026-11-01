# frontend —— 通知中心页面层(基线骨架)

由 gateway 同源提供的通知中心页面。基线只含静态骨架位置与契约,不含成品 UI。

## 固定交付边界

- 页面能力:通知列表、未读徽标(数字)、点击条目标读。
- 数据一律走同源网关 `/api/notifications*`,并显式携带请求头 `X-User: e2e-user`;
  **不得**直连 api 层,不得内置静态假数据。
- 验收口径(种子 n1/n2/n3 下):
  - 首屏未读计数 2;
  - 点击一条未读后计数变 1;重复点击同一条仍为 1;
  - 列表读取态随标读更新;
  - 刷新后本进程内状态保持(计数仍 1)。

## 模块位置契约

- `src/app.js`:页面逻辑模块位置(基线 NotImplementeds 占位);交付实现经同源
  网关拉取/标读并填充 `index.html` 的 `[data-testid="notification-center"]` 容器。
  本文件同时是聚合索引(codegraph)的最小可索引 JS 锚——成员仓无任何可索引
  文件会使聚合索引 member coverage 失败,阻塞 story 生成。

## 静态资源契约

- 入口:`index.html` + 原生 JS(无构建步骤、无 npm 依赖)。
- 由 gateway 以 `FRONTEND_ROOT` 托管;本地预览也必须经 gateway,不得绕行。

## 技术栈

原生 HTML/JS,零构建、零依赖。
