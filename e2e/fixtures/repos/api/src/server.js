// api 层模块位置契约(基线占位,非业务实现)。
// 交付实现:读取 BUSI_ROOT 组合 busi 领域层,消费 NOTIFICATION_SEED_PATH,
// 在 HOST/PORT 上用原生 node:http 暴露 README.md 的三条路由;
// 端口 0 时必须打印实际监听 URL。
export function createApiServer() {
  throw new Error("NotImplemented: 通知 API 服务由交付实现填充(基线仅契约)");
}
