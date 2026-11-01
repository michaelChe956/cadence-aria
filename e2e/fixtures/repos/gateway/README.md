# gateway —— 网关层(基线骨架)

把浏览器同源请求转发到 api 层,并承载 frontend 静态资源。基线只含契约与模块位置。

## 固定交付边界

- 转发路由(与 api 一一对应,前缀 `/api`):
  `GET /api/notifications*`、`GET /api/notifications/unread-count*`、`POST /api/notifications/{id}/read*`。
- `X-User` 请求头必填:缺失一律 401。
- 用户上下文以 `X-User` 为准转发,**不得**被查询参数中的 `user` 覆盖(X-User 覆盖向内 user 参数)。
- body 与状态码透传;不新增重试、限流或遥测逻辑。
- 同时服务 `FRONTEND_ROOT` 下的静态资源,使浏览器与 API 同网关 origin。

## 启动契约

- 入口:`node src/server.js`。
- 环境:`API_BASE_URL`(api 层基址)、`FRONTEND_ROOT`(frontend 仓 checkout 根)、`HOST`/`PORT`
  (端口 0 时须打印实际监听 URL)。零安装启动。

## 技术栈

Node 原生 ESM + 原生 `node:http`,零 npm 依赖。
