# 本地服务部署与启动规则（4317 共享实例）

日期：2026-10-30
状态：已启用（强制）
触发来源：一次真实故障——重发 4317 时我直接跑了 `script -qfc ".../aria web --port 4317"`，**绕过了唯一权威启动脚本 `/tmp/aria-deploy/start.sh`**，导致该脚本导出的 `ARIA_API_TRUSTED_ORIGINS` 没有进入进程环境。owner 一访问就报 `[request origin not allowed]`，服务「起来了」但**不可用**。同批还踩到两个相邻陷阱：`strings` 抽不出二进制里的中文串（误判「新代码没编进去」）、`git status` 看不见**空目录**（误判「产物没造出来」）。

## 核心原则

> **4317 是 owner 与端到端测试共用的共享实例。启动它只有一个权威入口：`/tmp/aria-deploy/start.sh`。
> 「端口在听」不等于「服务可用」——环境变量、TTY、前端资源、指纹自检缺一不可。**

> **2026-11-09 补充（第二次同款故障）**：本规则的**源**已收进仓内
> `scripts/aria-web-start.sh`（origin 白名单 / TTY / 停旧实例三件事都在里面）。
> `/tmp/aria-deploy/start.sh` 只是它的**安装副本** —— 每次启动前先
> `install -m755 scripts/aria-web-start.sh /tmp/aria-deploy/start.sh`，
> 避免「脚本在 /tmp、机器一重启就没了、谁也说不清白名单当时是什么」。

## 规则 1：只经唯一权威启动脚本启动，禁止手工拼命令行

**唯一权威入口**：`/tmp/aria-deploy/start.sh`（仓内源 = `scripts/aria-web-start.sh`；
内含 origin 白名单 + TTY 分配 + 停旧实例）。

```bash
# ✅ 正确（先安装仓内源，再启动）
install -m755 scripts/aria-web-start.sh /tmp/aria-deploy/start.sh
cd /tmp/aria-deploy && bash /tmp/aria-deploy/start.sh

# ❌ 禁止：绕过脚本直接起二进制 → origin 白名单丢失 → [request origin not allowed]
script -qfc "/tmp/deploy-target/release/aria web --port 4317" /tmp/aria-deploy/session.log

# ❌ 也禁止：用 nohup/setsid 手工后台起二进制 → 没有 TTY（鉴权 token 无法交付），
#    且 `script -qfc` 在无 TTY 时 stdin 读到 EOF 会连带把子进程带走（实例秒退）
setsid nohup /path/aria web --port 4317 >> /tmp/aria-deploy/session.log 2>&1 &
```

1. `start.sh` 负责三件事，**任何一件都不能丢**：导出 `ARIA_API_TRUSTED_ORIGINS`（远端域名 + 本机各 dev 端口白名单）、`pkill` 停旧实例、`exec script -qfc` 分配 TTY。
2. 需要改 origin 白名单或端口 → **改脚本**（仓内源 `scripts/aria-web-start.sh`），不要改启动命令行。
3. 诊断 origin 拒绝：`tr '\0' '\n' < /proc/<aria_pid>/environ | grep ARIA_` —— 应该是**空**（=没设 = 就是这个原因）。
4. **二进制必须显式指向本轮开发的那份**：`ARIA_BIN=<本轮 worktree 或 /tmp/deploy-target 的件>`。
   脚本默认 `/tmp/deploy-target/release/aria`；开发期可用
   `ARIA_BIN=<worktree>/target/debug/aria`。
   ⛔ 禁止沿用**别的 worktree** 的旧二进制（曾发生：实例跑的是 `bugfix-b-11-02` 的件，
   本轮修复在另一个分支 ⇒ 点【更新】仍走旧逻辑 ⇒ 看起来像「修复没生效」）。
   启动后用自检第 5 项确认「跑的就是 ARIA_BIN」。

## 规则 2：TTY 强制（鉴权 token 靠 TTY 交付）

1. 必须经 `script -qfc`（`start.sh` 已含）启动；**非 TTY 启动时 T1 鉴权 bootstrap token 无法交付**。
2. 启动后确认日志首行带 TTY 标记：`Script started on ... TTY="/dev/pts/N"`。
3. 进程用 harness 的 service + `pty: true` 托管，便于 stop/restart/读日志。

## 规则 3：构建顺序（前端先于二进制）

本仓前端资源由 `rust-embed` **编译期嵌入** `web/dist`（`build.rs` 检查 `web/dist/index.html`，缺失即 panic）。

```
pnpm -C web build                     ← 必须先做
CARGO_TARGET_DIR=/tmp/deploy-target cargo build --release --locked
```

1. 改了 `web/src/**` → **必须**重建 dist **并**重建二进制；只重建二进制等于没发。
2. 构建用宿主机 Rust（见 `build-test-commands.md`），`CARGO_TARGET_DIR=/tmp/deploy-target` 与测试用的 `target/` 分开，避免污染与重复编译。
3. cargo 编译窗口串行：所有 cargo 命令带 `flock /tmp/aria-cargo.lock`。
4. **发布态件** = `/tmp/deploy-target/release/aria`（脚本默认）；**开发期件** =
   `<本轮 worktree>/target/debug/aria`（`ARIA_BIN=` 指定）。两者都必须在
   `pnpm -C web build` 之后构建，否则内嵌的是旧 dist。

## 规则 4：回滚件先备份

替换二进制**之前**先备份旧件（出问题时唯一的退路）：

```bash
cp /tmp/deploy-target/release/aria /tmp/aria-rollback-$(sha256sum /tmp/deploy-target/release/aria | cut -c1-16)
```

回滚方式：把回滚件 `cp` 回 `/tmp/deploy-target/release/aria`，再按规则 1 重启。

## 规则 5：重启前确认实例空闲（共享实例纪律）

4317 上可能正在跑 owner 的端到端测试。重启前：

```bash
ls -la --time-style=long-iso /tmp/aria-deploy/session.log   # 最后写入时间
date '+%Y-%m-%d %H:%M:%S'                                    # 现在
```

最后写入时间距今超过几分钟 = 空闲。**若明显正在使用，先告知 owner 再重启** —— 打断别人的测试是事故。

## 规则 6：启动后自检 6 项（缺一不可）

> 自检已脚本化：`bash /tmp/aria-deploy/start.sh --check`（口径与下表逐项一致；
> 指纹扫描用 `ARIA_FINGERPRINTS`（每行一个稳定字符串）+ `ARIA_ROLLBACK`（回滚件路径）传入）。

| # | 检查 | 命令 / 判据 |
|---|---|---|
1 | 端口在听 | `ss -ltnp \| grep 4317` |
2 | 全站可用 | `curl -o /dev/null -w "%{http_code}" http://127.0.0.1:4317/` → 200；`/api/health` → 200 |
3 | 启动自证 | session 日志含 `aria web listening on http://127.0.0.1:4317` 与 `governance_complete=true` |
4 | 环境变量在位 | `tr '\0' '\n' < /proc/<pid>/environ \| grep ARIA_API_TRUSTED_ORIGINS` 非空 |
5 | **跑的是新件** | 见下方指纹扫描 |
6 | 新代码确实在件里 | 见下方指纹扫描 |

### 指纹扫描：用 `grep -a`，不要用 `strings`

```bash
# ✅ 对：二进制安全，能抽到 UTF-8 中文
grep -a -c -- "写入范围（硬边界）" /tmp/deploy-target/release/aria

# ❌ 错：strings 默认只抽 ASCII 序列 → 中文串抽不到 → 会误判「新代码没编进去」
strings /tmp/deploy-target/release/aria | grep "写入范围"
```

指纹取**本次改动的稳定字符串**：常量名、段标题、固定文案。**必须同时对照回滚件**（旧件应全部「不在」），否则无法区分「编进去了」与「本来就有」。

## 故障签名 → 处置（先认签名，再动手）

| 症状 | 根因 | 处置（**唯一**路径） |
|---|---|---|
| 页面报 `[状态读取失败：request host not allowed]` / `forbidden_host` / `forbidden_origin`，但端口在听 | 手工直起二进制 ⇒ `ARIA_API_TRUSTED_ORIGINS` 没进进程环境 | `tr '\0' '\n' < /proc/<pid>/environ \| grep ARIA_` 确认为空 → 经 `bash /tmp/aria-deploy/start.sh` 重启；⛔ 禁止再手工拼命令「补一把」 |
| 端口在听但页面 404 / 白屏 | `web/dist` 没随二进制重建（rust-embed 编译期嵌入） | `pnpm -C web build` → `cargo build` → 经脚本重启；用 `curl -s http://127.0.0.1:4317/ \| grep -oE 'assets/index-[A-Za-z0-9_-]+\.js'` 与 `ls web/dist/assets/` 比对 |
| 界面看着「修复没生效」（如【更新】点了没反应） | 实例跑的是**别的 worktree** 的旧二进制 | 自检第 5 项比对 `/proc/<pid>/exe` 与 `ARIA_BIN`；用 `ARIA_BIN=<本轮 worktree 的件>` 经脚本重启 |
| 启动后实例秒退 | 用 `nohup/setsid` 手工后台起，`script -qfc` 无 TTY 时 stdin EOF 把子进程带走 | 改用 harness 具名长驻服务 + `pty: true` 托管脚本 |

## 违反判定

出现下列任一情形，视为**部署未完成，不得声称已重发**：

- 绕开 `start.sh` 手工拼启动命令行 → origin 白名单丢失
- 用 `nohup`/`setsid` 手工后台起二进制 → 无 TTY（鉴权 token 无法交付 + 实例秒退）
- 启动的二进制不是本轮开发的那份（`ARIA_BIN` 未指向本轮件）→ 界面看起来「修复没生效」
- 未带 TTY 启动 → 鉴权 token 无法交付
- 改了 `web/src/**` 但未重建 dist / 未重建二进制 → 页面仍是旧资源
- 只验证「端口在听」就宣布完成 → 可能环境变量缺失、前端资源陈旧、跑的是旧件
- 用 `strings` 判定中文指纹是否存在 → 误判
- 未备份回滚件就替换二进制 → 无退路
- 未确认实例空闲就重启 → 打断 owner 的在跑测试

## 检查清单（重发后逐项打勾）

- [ ] 经 `bash /tmp/aria-deploy/start.sh` 启动（未手工拼命令行；未用 nohup/setsid）
- [ ] 启动前已 `install -m755 scripts/aria-web-start.sh /tmp/aria-deploy/start.sh`（仓内源即真源）
- [ ] `ARIA_BIN` 指向本轮开发的那份二进制（自检第 5 项核对 `/proc/<pid>/exe`）
- [ ] 启动日志首行含 `TTY="/dev/pts/..."`
- [ ] `ss -ltnp` 4317 在听；`GET /` 与 `/api/health` 均 200
- [ ] 日志含 `governance_complete=true`
- [ ] `/proc/<pid>/environ` 里 `ARIA_API_TRUSTED_ORIGINS` 非空
- [ ] `grep -a` 指纹：新件命中本次改动的字符串，**回滚件同一批全部不命中**
- [ ] `web/dist` 已重建（若本次改了 `web/src/**`）
- [ ] 回滚件已备份到 `/tmp/aria-rollback-<sha>`
- [ ] 重启前已确认实例空闲（session 日志最后写入时间）
- [ ] 已把「怎么测、看哪几项」告诉 owner
