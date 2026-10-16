# Bug 报告: filesystem audit 不感知 gitignore,导致正常前端仓无法进入 coding

- 报告人: 测试会话 (2026-10-16)
- 严重级: 设计缺陷 / 阻断前端仓 coding
- 状态: 已临时绕过(手动清理),待主会话定夺

## 现象

平台 coder 启动时报错并 fail-closed 拒绝工作:

```
[root recipe filesystem audit failed: product_store_io: audit snapshot budget exceeded at /tmp/test-center-java/frontend/node_modules/element-plus/es/components/virtual-list/src/builders/build-grid.mjs.map: 67121989 cumulative bytes > max_bytes 67108864 (fail-closed)]
```

## 根因(设计层面)

审计/快照按磁盘实际字节累计,预算 64MiB,不感知 gitignore:
- `node_modules/`(127M)在 `frontend/.gitignore` 中且未被 git 追踪,但文件在磁盘上就被计入
- `busi/target`(45M)、`gateway/target`(31M)同理(fat jar + .original)
- 任何常规前端仓(node_modules 是标配)或构建过的 Java 仓都会撞线

## 影响

1. 前端仓带依赖即无法 coding;用户被迫删除 node_modules/target,破坏本地联调环境(vite dev 直接失效)
2. 平台 coder 完工后若重建了构建产物,下一条 coding 前又要人工清理,体验断裂

## 期望行为

- 审计/sandbox 快照默认 gitignore-aware:解析仓库 `.gitignore`(及 `.git/info/exclude`),被忽略路径不计入预算、不进快照
- 或至少提供配置:忽略路径模式 / 预算上限可调
- fail-closed 前给出可操作的提示(当前报错只给了超限文件路径,没说明可通过忽略/清理绕过)

## 复现步骤

1. `/tmp/test-center-java/frontend` 执行 npm install(缓存 `/tmp/test-npm-cache` 在,约 20s 恢复 node_modules)
2. 平台对该仓发起 coder 启动
3. 观察上述报错

## 现场数据

| 时点 | 目录体积 | 结果 |
|---|---|---|
| 带 node_modules + target | 204M | fail-closed |
| 清理后(仅源码) | 672K / 26 文件 | 可过 |

清理动作: `rm -rf frontend/node_modules frontend/dist busi/target gateway/target`(均已在 gitignore 内,三仓 git status 无变化)。
