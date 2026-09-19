import type { ProxyOptions } from "vite";

// 图片生成（gpt-image-2）可能耗时数分钟，dev 代理默认 proxyTimeout 较短会提前掐断请求。
// 这里放宽到 10 分钟，与后端 ImageClient 的 600s 超时对齐（仅影响开发环境代理）。
const LONG_RUNNING_PROXY_TIMEOUT = 600_000;

// T4 门禁排期注：4317 与 T1 门重测部署冲突时以 ARIA_E2E_PORT 旁路（默认不变）。
const apiTarget = `http://127.0.0.1:${process.env.ARIA_E2E_PORT ?? "4317"}`;

export const devServerProxy: Record<string, ProxyOptions> = {
  "/api": {
    target: apiTarget,
    ws: true,
    timeout: LONG_RUNNING_PROXY_TIMEOUT,
    proxyTimeout: LONG_RUNNING_PROXY_TIMEOUT,
  },
  "/ws": {
    target: apiTarget,
    ws: true,
  },
};
