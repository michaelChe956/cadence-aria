import { test } from "node:test";
import assert from "node:assert/strict";
import port from "../lib/port.js";
import net from "node:net";

const { pickFreePort } = port;

test("pickFreePort returns a usable port", async () => {
  const p = await pickFreePort();
  assert.ok(Number.isInteger(p) && p > 0 && p < 65536, `端口非法: ${p}`);
  // 验证该端口当下可绑定
  await new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.once("error", reject);
    srv.listen(p, "127.0.0.1", () => srv.close(resolve));
  });
});

test("two consecutive picks both usable", async () => {
  const a = await pickFreePort();
  const b = await pickFreePort();
  assert.ok(a > 0 && b > 0);
});
