// gateway 层模块位置契约(基线占位,非业务实现)。
// 交付实现:按 README.md 转发 /api/notifications* 三条路由(X-User 必填、
// 缺失 401、X-User 覆盖向内 user 参数、body/状态码透传),
// 并托管 FRONTEND_ROOT 静态资源;端口 0 时打印实际监听 URL。
export function createGatewayServer() {
  throw new Error("NotImplemented: 通知网关由交付实现填充(基线仅契约)");
}
