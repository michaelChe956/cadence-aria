# 观察: coder 沙箱无运行时,CHECK 验证任务系统性不可执行(附方案建议)

- 报告人: 测试会话 (2026-10-16)
- 现场: test-java/project_0002 issue_0001 coding attempt `coding_attempt_dd5984e37b5848288c99dce26ab16dea`
- 影响: 每个 WI 的构建/运行类 CHECK 都会卡三按钮分诊卡,需人工介入

## 现象

busi WI coder 交付代码后卡验证门:
```
CHECK-001 依赖的容器运行时在本执行环境不可用（podman 在配置阶段即失败），
宿主亦无 JDK 与 Maven、无 Maven 依赖缓存、无外网；CHECK-002/003 依赖运行中的
busi 服务与 MySQL，同样不可用。实现代码已完整交付并提交。
```

## 根因(源码证据;已按 design-task 复核修正)

> 勘误(2026-10-16): 初版两处错误——①「kimi_code → codex_provider 适配」不成立,
> kimi 有独立 `kimi_code_provider/`;②「landlock/seccomp 过滤」不成立,那是 codex provider 的机制。
> 正确机制见下及《2026-10-16_分析-pi与kimi执行环境差异》。

- coder(kimi_code,独立 `kimi_code_provider/`)的命令走 Aria 宿主 ACP `terminal/*` 服务,
  被强制套 bwrap 沙箱: `--ro-bind / /` + `--unshare-net` + `--clearenv` + no-new-privs
  (`kimi_code_provider/.../sandbox.rs:450-465`,Auto 模式无 bwrap 直接拒绝)
  → podman 结构性不可用(`/run/user/1000/libpod` 只读、newuidmap 无法提权),宿主无 JDK/Maven,网络隔离依赖拉不动
- git 由 aria 代管(git-operation.json),coder 能力面 = 读仓 + 写 worktree
- CodexSandboxMode 两档(REQ-LCG-04)仅约束 codex provider,与 kimi/pi 无关

这是设计而非缺陷(安全边界)。但 plan 的 CHECK 验证任务(构建/起服务/打接口)在 coder 侧
**物理不可执行**,只能靠宿主人工补做。

## 本次宿主补验记录(busi WI, 全部通过)

- 构建: maven 容器内副本构建(不动 worktree,避免撑爆 64MiB audit 预算), jar 产出成功
- 运行: 临时容器(18099)连 test-mysql,分页契约/启动迁移/分类 CRUD/置顶/详情/编辑/删除/404 边界/mysql 表变更全过
- coder 交付质量本身合格: 提交 `204ca26 feat(busi): 公告分类域改造与公告接口补全`

## 建议(方案 B: 受控验证 runner)

CHECK 在 plan 里预声明命令白名单(如 `mvn -B package -DskipTests`);coder 卡验证门时,
aria 在宿主 podman 代跑(预配镜像 + maven/npm 缓存卷 + 网络白名单仅 maven-central/npm-registry),
stdout + exit code 回填 attempt 作为证据,自动判定通过/失败。

信任边界不变: 命令是 plan 预声明(非 coder 输入),容器隔离,网络白名单。
替代方案 A(现状): CHECK 标注「宿主验证代理执行」,分诊卡出现时由人工补做回贴——每个 WI 一次人工介入。
不推荐方案 C(沙箱内嵌容器运行时): 违背 REQ-LCG-04,扩大攻击面。

## 关联

- 同会话早前报告: filesystem audit 不感知 gitignore(bug-filesystem-audit) —— 同一个「coder 环境
  与真实工程环境不匹配」主题,建议批2 一并考虑
