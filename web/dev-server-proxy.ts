import type { ProxyOptions } from "vite";

export const devServerProxy: Record<string, ProxyOptions> = {
  "/api": {
    target: "http://127.0.0.1:4317",
    ws: true,
  },
  "/ws": {
    target: "http://127.0.0.1:4317",
    ws: true,
  },
};
