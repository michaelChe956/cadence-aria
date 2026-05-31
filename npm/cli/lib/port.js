"use strict";

const net = require("node:net");

// 让内核分配一个空闲端口（listen 0），取到后立即释放并返回端口号。
// 存在「释放后到二进制再 bind」的微小窗口，但本机本地场景概率极低，且二进制 bind 失败会显式报错。
function pickFreePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.once("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close((err) => (err ? reject(err) : resolve(port)));
    });
  });
}

module.exports = { pickFreePort };
