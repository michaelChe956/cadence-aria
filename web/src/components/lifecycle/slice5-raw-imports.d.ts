// 片5 源码级扫描测试的类型支撑：`web/tsconfig.json` 没有配置 `types`
// （也没有 vite/client），因此 Vite 的 `?raw` 模块在此最小声明一次。
// 只影响类型检查，不改变任何构建/运行行为。
declare module "*?raw" {
  const content: string;
  export default content;
}
