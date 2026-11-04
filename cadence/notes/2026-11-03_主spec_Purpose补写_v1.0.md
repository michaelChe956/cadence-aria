# 主 spec Purpose 补写（logical-codebase-audit-budget / provider-cli-installation）

> 工作目录：`.worktrees/bugfix-b-11-02`
> 背景：块2 归档的两份 delta 直接以 `## ADDED Requirements` 开头、无 `## Purpose` 段，`openspec archive` 同步时无从复制，主 spec 只能留 TBD 占位。本次仅补写这 2 个文件的 `## Purpose`（各替换第 4 行占位行），requirement / 场景 / 编号零改动。

## 1. logical-codebase-audit-budget — Purpose 全文

> 约束逻辑代码库聚合初始化中文件快照审计的预算口径与超限失败面。快照审计用于证明沙箱 recipe 未写到 allowlist 之外，其预算以**本次变更量**为口径而非整仓存量体量（全仓两遍只记元数据，仅对变更文件读取内容），使 `.git` 与构建产物存量巨大的老仓不再压垮冷启动预检；`.git` 子树整棵不入快照，与聚合索引的排除口径统一；内容读取流式计费、越限即停；预算超限时给出带大项分组与成员仓归属的中文可行动指引，英文机器行形态保持稳定。

**取材来源**：

| 来源 | 节 | 采信点 |
|---|---|---|
| `openspec/changes/archive/2026-11-03-fix-aggregate-cold-start-and-audit-budget/specs/logical-codebase-audit-budget/spec.md` | REQ-AUD-01～04 正文与场景 | 能力定义主体：变更量口径 / 两遍元数据差分、跳过整个 `.git/`、流式读取、结构化超限失败面（大项分组 + 成员仓归属 + 中文指引 + 英文机器行稳定码） |
| 同 change `design.md` | 决策 8（K-1 / K-2 / K-3 §3.5） | 为什么存在：老仓 `.git` 300 MB~1 GB + 构建产物存量令 64 MiB 预算余量≈0、冷启动被压垮；K-2 与 codegraph `**/.git/` 排除口径统一；K-3 元数据差分法；owner 决策 1/4/6/7 |
| `cadence/designs/2026-11-02_技术方案_块2_聚合冷启动与审计预算_v1.0.md` | §3.5（3.5.1 owner 决策 / 3.5.4 K-3 / 3.5.5 流式读 / 3.5.7 指引去掉 `/.git/` 行） | 口径细节交叉印证 |
| 同 change `proposal.md` | 目标表 #5 / #6 | 「审计预算不再被存量体量压垮（跳过 `.git/`、按本次变更量计、老仓可用）」「文案可行动（中文 + 点名成员仓与大项 + 两条出路）」 |

## 2. provider-cli-installation — Purpose 全文

> 为四家 provider CLI（Claude Code、Codex、Pi、Kimi Code）与 codegraph 提供平台内的安装与更新能力，让「未安装 / 版本太低」这一冷启动最常见的用户侧失败成因能在界面内闭环处理。安装入口放在能力探针处，由探针结果与本地状态共同决定显示「安装」还是「更新」；npm 类对象（含 codegraph）走镜像源可配置的 npm 包，Kimi Code 走执行前明示来源的官方脚本；安装以异步 durable 作业执行、完成后重新探测版本与可用性；失败按成因分类并给手动命令兜底，平台不自动提权；安装成功不等于能力已核验，仍需既有核验通道确认。

**取材来源**：

| 来源 | 节 | 采信点 |
|---|---|---|
| `openspec/changes/archive/2026-11-03-fix-aggregate-cold-start-and-audit-budget/specs/provider-cli-installation/spec.md` | REQ-PCI-01～06 正文与场景 | 能力定义主体：五类对象安装源、按钮显示规则、npm 镜像与安装形态、版本判定与 durable 缓存、异步作业 + 装完重探、失败分类 + 手动命令 + 不自动 sudo |
| 同 change `design.md` | 决策 9（§3.6）+ owner 决策记录 #10～#16 | 为什么存在：「没安装」是失败面最常见的用户侧原因，平台此前不提供安装能力；codegraph 纳入同一安装机制（#15）；Kimi 明示脚本来源（#16）；不自动 sudo（#12）；安装 ≠ 核验 |
| `cadence/designs/2026-11-02_技术方案_块2_聚合冷启动与审计预算_v1.0.md` | §3.6（3.6.1 安装源 / 3.6.5 按钮规则 / 3.6.6 异步作业 / 3.6.7 失败兜底） | 机制细节交叉印证 |
| 同 change `proposal.md` | 背景节 | 冷启动三步全红、`provider command not found` 的动机背景（proposal 非目标中「不做自动安装」指不自动触发/不自动提权，与最终「用户点按钮 + 手动命令兜底」的口径一致，非矛盾） |

## 3. 验收输出

### `npx --no-install openspec validate --specs`

```
…（逐项 ✓，共 78 项）…
✓ spec/logical-codebase-audit-budget
✓ spec/provider-cli-installation
…（elided）…
Totals: 78 passed, 0 failed (78 items)
```

期望 78 passed / 0 failed —— **达成**。（完整原始输出：artifact://9739）

### `grep -rn 'TBD' openspec/specs/`

```
No matches found
```

期望 0 命中 —— **达成**，specs 目录已无任何 TBD，无需上报其它残留。

## 4. 改动面

- `openspec/specs/logical-codebase-audit-budget/spec.md`：仅第 4 行（TBD 占位 → Purpose 段落）
- `openspec/specs/provider-cli-installation/spec.md`：仅第 4 行（TBD 占位 → Purpose 段落）
- delta / 归档目录 / 代码 / tasks.md / spec 正文·场景·编号：零改动；未 git add/commit/stash/checkout；未跑 cargo test

## 5. Concerns

1. validate 中 `human-gate-convergence` 有一条 WARNING（requirement 文本 >500 字符，建议把示例挪进场景）——**既有问题**，不在本次 2 文件范围内，未动。
2. 归档 `proposal.md` 的非目标写「不做 provider CLI 的自动安装（属块5 提案）」，而本 change 最终交付了安装/更新能力：两者不矛盾（「自动安装」= 未经用户触发/自动提权的安装；交付物是用户显式点击 + 失败给手动命令、不自动 sudo，owner 决策 #12），属草案范围到 owner 拍板（#10～#16）的演进；归档目录按约束不改，仅在此备注口径。
