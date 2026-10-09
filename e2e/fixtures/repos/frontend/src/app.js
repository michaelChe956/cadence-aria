// frontend 层模块位置契约(基线占位,非成品 UI)。
// 交付实现:经同源网关 /api/notifications*(fetch 显式携带请求头 X-User: e2e-user)
// 拉取列表与未读计数,渲染列表/未读徽标,点击条目调用标读接口并更新读取态;
// index.html 的 [data-testid="notification-center"] 容器由本模块填充。
// 基线不携带静态假数据,不直连 api 层,不引入构建步骤与 npm 依赖。
// 注:本文件同时是 codegraph 可索引的最小 JS 锚(聚合索引成员覆盖前置)。
export function createNotificationCenter() {
  throw new Error("NotImplemented: 通知中心页面由交付实现填充(基线仅契约)");
}
