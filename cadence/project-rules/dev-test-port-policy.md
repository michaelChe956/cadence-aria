# 开发期测试端口与发布态回归端口规则（强制）

状态：已启用（强制）
触发来源：owner 2026-11-06 裁定 —— 4317 共享实例的前端资源由 `rust-embed` 在 **`cargo build` 那一刻**把 `web/dist` 烤进二进制（`src/web/static_assets.rs:15`）。开发期改了前端但没重建 dist ⇒ 4317 呈现**过期 UI**，与 vite(5173) 的当前源码不一致 ⇒ owner 看到「两套 UI」会误判成「前端写错了」。

> 实测判据（2026-11-06）：4317 内嵌 `assets/index-B961qTvE.js`，而当前 `web/dist/assets/` 里已无此文件 ⇒ 结论「dist 过期」，不是代码错。

---

## 核心原则（三条，不可协商）

1. **开发期一律用 5173 测**（vite dev server，直接服务 `web/src` 当前源码 ⇒ 改完刷新即生效，**不需要重建 dist、不需要重发二进制**）。
2. **4317 只做「发布态回归」**：开发期问题全部测完之后，owner 用 4317 再走一遍回归 ⇒ 验证「重建后的 dist → 内嵌资源 → origin 白名单 → 生效栈」这条**发布链**是否完好。
3. **两个端口都在跑时，URL 即端口**（5173 = 开发态、4317 = 发布态）⇒ 任何汇报、取证、缺陷单**必须写清是哪个端口**。

---

## 具体条款

### 开发期（5173）

- 前端入口：`http://127.0.0.1:5173`；后端 API 仍是 `http://127.0.0.1:4317`（vite 的 `/api` 代理指向它）⇒ **前端是活的，API 不变**。
- ⛔ **不许**用 4317 的页面判断「前端改没改对」⇒ 它可能是旧 dist 烤进去的快照。
- ⛔ 不许在开发期反复 `cargo build` + 重启 4317 来「让前端生效」⇒ 那是发布动作，不是开发动作。
- 开发期发现的问题**逐条记录**（端口 + 操作 + 现象 + 判据），供 4317 回归时复测。

### 发布态回归（4317）

- **触发条件**（三者都满足才做）：
  1. 开发期（5173）测试清单**全部通过**；
  2. 本轮有 `web/src/**` 改动 ⇒ 必须重建 `web/dist`；
  3. 本轮有 Rust 改动 ⇒ 必须重建二进制并重启。
- **构建顺序不可颠倒**（rust-embed 编译期嵌入）：
  ```
  pnpm -C web build        →  cargo build --locked   →  只经 /tmp/aria-deploy/start.sh 重启
  ```
  ⇒ ⇒ 顺序错了 = 内嵌旧 dist = 「两套 UI」。
- 重启前：备份回滚件到 `/tmp/aria-rollback-<sha>`；看 session 日志最后写入时间确认实例空闲。
- 重启后必跑 `cadence/project-rules/local-service-deploy.md` 的 6 项自检（端口在听 / `GET /` 200 / `GET /api/health` 200 / `governance_complete=true` / `ARIA_API_TRUSTED_ORIGINS` 非空 / 跑的是新件）。

### 两端口 UI 不一致时的排查顺序（先取证再下结论）

```
1. 取 4317 内嵌资源名：curl -s http://127.0.0.1:4317/ | grep -oE 'assets/index-[A-Za-z0-9_-]+\.js'
2. 与当前 dist 比对：ls web/dist/assets/
3. 不一致 ⇒ dist 过期 ⇒ 重发即可，不是代码错
4. 一致却仍长得不同 ⇒ 才去查代码/缓存（浏览器强刷、vite 缓存）
5. 指纹扫描用 grep -a，不要用 strings（只抽 ASCII，抽不到中文串）
```

---

## 开发期特有故障签名：Fast Refresh 打进来的 hooks 错位（2026-11-09 实测）

**签名**：页面弹「Something went wrong!」+ `Rendered more hooks than during the previous render`
（或 `Rendered fewer hooks than expected`），且**新开标签不复现**。

**根因**：5173 的 vite dev server 直接服务 `web/src` 源码，浏览器标签长期开着时，
**同一 worktree 里其它会话的改动会经 HMR 热替换进已挂载的组件**。当那次改动
**增删了 hooks**（例如把一段逻辑抽成新 hook、或给组件加 `useState`/`useEffect`），
React Fast Refresh 会把新的 hook 列表套到旧实例上 ⇒ hooks 数量前后不一致 ⇒ 报上面的错。
这是**开发态热更新产物**，不是产品缺陷。

**取证（先分清是不是它）**：

```
1. grep -nE "hmr update|page reload" <vite 日志>   # 报错时刻附近是否有 hmr update
2. 新开一个标签复现同一操作                        # 不复现 ⇒ 就是 Fast Refresh 产物
3. 4317（发布态、新标签）做同一操作                # 不复现 ⇒ 发布链完好
```

**处置**：**硬刷新（Ctrl+Shift+R）**即可，不需要改代码、不需要重建 dist、不需要重发二进制。
硬刷新后仍复现 ⇒ 才按真实缺陷排查（此时取 console 的组件栈）。

**预防**：
- 共享 worktree 上多会话并行编辑时，**开发端口只服务一个 worktree**；发布态验证一律用 4317 新标签。
- 看到 vite 日志里有 `hmr update` 而你正在做验收 ⇒ 先硬刷新再验收。

**真实事故记录**：2026-11-09 17:07，owner 点 provider【更新】后页面炸 hooks 错位；
同期 vite 日志显示 `IssueLifecycleWorkbench.tsx` / `IssueLifecycleWorkbenchView.tsx`
在 17:05–17:08 被连续 hmr update（另一会话的知识库重构**新增了 hooks**），
而 provider 安装完成的健康刷新恰好触发该组件重渲染 ⇒ 错位显形。
新开标签在 4317 与 5173 各跑通一次真实更新（claude / codex / pi），**均无此错**。

---

## 边界（不替代其它规则）

- 本规则只约束「**测哪个端口**」与「**何时做发布态回归**」。
- 启动方式仍以 `cadence/project-rules/local-service-deploy.md` 为准（只经 `/tmp/aria-deploy/start.sh`，必须带 TTY；该脚本的仓内源 = `scripts/aria-web-start.sh`）。
- 界面验证方式仍以 `cadence/project-rules/browser-verification-only.md` 为准（内置浏览器、真实渲染 + 真实点击）。
- origin 白名单仍以 `scripts/aria-web-start.sh` 的 `DEFAULT_ORIGINS` 为唯一真源（改脚本，不改命令行）。
