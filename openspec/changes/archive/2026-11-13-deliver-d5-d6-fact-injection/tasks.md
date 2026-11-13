# Tasks: deliver-d5-d6-fact-injection（REQ-WSC-06 拆分件）

> 本件是**规范同步件**：其工作包全部为「析出 + 落档 + 校验」，**不改任何产品源码**。产品实现由 `rework-prompt-input-surface` 的 §5 / §6 / §7 / §8 已勾选工作包承载，其证据见 `cadence/analysis-docs/2026-11-10_分析报告_WIG基线树事实注入与前轮状态复算_复测_v1.0.md` §9.1 与 `cadence/analysis-docs/2026-11-11_问题简报_WIG三缺陷待修_v1.0.md` §1。

## 0. 析出与核对

- [x] 0.1 **已交付场景权威清单**：从 `rework-prompt-input-surface/tasks.md` 的「requirement 映射」条目逐条取出（5.5 / 6.7 / 7.8）⇒ **8 个场景**：越界路径回灌最近候选 / 基线树事实直供 / 基线树清单不得占用固定规范面预算 / 注入前轮结构化清单 / 重复判定不靠记忆 / 前轮 finding 状态求交按内容身份 / 前轮 finding 分区注入 / 前轮清单同身份去重。验证：清单与已勾选工作包的映射行逐字一致
- [x] 0.2 **场景正文核验**：8 个场景正文逐条检查，确认**不含**未实施义务（L2 依赖图事实 / 逐仓实况 / gaps-only 投影最小内容 / 返修轮逐字节重发 / 成员仓摘要名额分配 / 分级供给）⇒ 零命中。验证：逐条关键词扫描输出
- [x] 0.3 **主规格原有场景保全**：主规格 REQ-WSC-06 的 **28 个场景标题与正文逐字**提取进本件 delta。验证：脚本比对，主规格侧零改动

## 1. 描述段压缩

- [x] 1.1 **只承载已交付义务**：描述段 = 主规格原有基线规则 + 「基线树事实直供」SHALL 强度 + 「前轮 finding 状态由平台按内容身份机械复算」SHALL 强度。验证：两项义务逐字在位
- [x] 1.2 **未交付义务零残留**：L2 依赖图事实直供 / reviewer 投影最小内容（gaps-only 等）/ 返修轮 delta-only 重发 三项**不进入**本件描述段。验证：关键词断言零命中
- [x] 1.3 **门禁余量**：描述段实测 **244 字符**（归档后主规格门禁 ≤500）。验证：python 实测长度贴入验收记录

## 2. 契约与校验

- [x] 2.1 **delta 形态**：本件 `specs/work-item-plan-single-candidate/spec.md` 为 **MODIFIED REQ-WSC-06** 单块；标题与主规格逐字相同；场景 = 28 原有 + 8 已交付 = **36**。验证：场景计数
- [x] 2.2 **change 级严格校验**：`openspec validate deliver-d5-d6-fact-injection --strict` **exit 0**。验证：真实命令输出贴入验收记录
- [x] 2.3 **四件齐备**：`proposal.md` / `design.md` / `tasks.md` / `specs/work-item-plan-single-candidate/spec.md` 齐备。验证：目录列举

## 3. 归档与同步

- [x] 3.1 **归档带 spec 同步**：本件以 `openspec archive deliver-d5-d6-fact-injection -y` 归档（**不**使用 `--skip-specs`）⇒ 主规格获得 D5/D6 的已交付口径。验证：归档输出「Applying changes to openspec/specs/work-item-plan-single-candidate/spec.md」
- [x] 3.2 **描述段门禁复测**：归档后 `openspec validate --specs --strict` 全绿；主规格 REQ-WSC-06 描述段实测 ≤500 字符。验证：真实命令输出与实测长度
- [x] 3.3 **主规格改动面收敛**：`git diff --stat -- openspec/specs/` 仅 REQ-WSC-06 一处。验证：diff --stat 输出
- [x] 3.4 **未实施部分的移交登记**：本件 design §5 已逐条登记 5 个未实施场景簇的「状态 + 落点」；`rework-prompt-input-surface` 归档件内另附同等登记。验证：两处登记均存在且口径一致

## 4. 与主体 change 的次序

- [x] 4.1 **次序**：本件（带同步）**先**归档 ⇒ `rework-prompt-input-surface`（`--skip-specs`）**后**归档 ⇒ 两者不同时为 active ⇒ MODIFIED REQ-WSC-06 所有权无双声明。验证：`openspec list` 中同一 requirement 不被两个 active change 声明
