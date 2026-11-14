// `failure_copy` 的码表与布局规则表（`include!` 拆出，保持两个文件都低于
// `large_file_guard` 的 1200 行硬线；符号仍在 `failure_copy` 模块命名空间）。
// 注意：本文件被 include! 插到模块中部，因此只能用 `//` 行注释——`//!` 内部
// 文档注释在模块中间是 E0753。

/// 六条布局规则与七个对外稳定码的归属（REQ-LAYOUT-01 / REQ-LAYOUT-02）。
///
/// 规则表与文案同处本模块：块3 只做呈现，不自建文案表、不复制中文。
pub struct LayoutRule {
    pub rule_zh: &'static str,
    pub codes: &'static [&'static str],
}

pub const LAYOUT_RULES: &[LayoutRule] = &[
    LayoutRule {
        rule_zh: "逻辑代码库根目录不能是 git 仓库",
        codes: &["aggregate_root_is_git"],
    },
    LayoutRule {
        rule_zh: "每个成员必须是逻辑代码库根的一层直接子目录（<LC根>/<成员>/）",
        codes: &[
            "aggregate_root_member_outside_root",
            "aggregate_index_layout_unsupported",
        ],
    },
    LayoutRule {
        rule_zh: "成员必须是 LC 根内的真实目录，不能经符号链接指向根之外",
        codes: &["aggregate_root_member_symlink_escape"],
    },
    LayoutRule {
        rule_zh: "一个聚合根只能归属一个逻辑代码库",
        codes: &["aggregate_root_overlap"],
    },
    LayoutRule {
        rule_zh: "成员必须是正常检出，不能是嵌套 worktree",
        codes: &["aggregate_root_nested_worktree"],
    },
    LayoutRule {
        rule_zh: "每个成员至少要有一个可索引的源码文件",
        codes: &["aggregate_index_member_coverage_failed"],
    },
];

fn layout_rule_of(key: &str) -> Option<&'static LayoutRule> {
    LAYOUT_RULES.iter().find(|rule| rule.codes.contains(&key))
}

/// 内部码 → 对外稳定码（对外码是 canonical key，内部码只作反查）。
const ALIASES: &[(&str, &str)] = &[
    (
        "member_path_outside_root",
        "aggregate_root_member_outside_root",
    ),
    (
        "member_symlink_escape",
        "aggregate_root_member_symlink_escape",
    ),
    ("nested_worktree", "aggregate_root_nested_worktree"),
    // 历史内部码（已无生产者，客户端兼容期仍需成品文案）。
    (
        "aggregate_root_ownership_conflict",
        "aggregate_root_overlap",
    ),
];

/// 码表条目：`(码, 问题, 原因, 怎么做, 分诊)`。
///
/// 模板里可用占位符：`{subject}`（可确认对象，缺省「该对象」）、
/// `{member}`（`「名字」` 或空串）、`{stage}`、`{provider}`、`{detail}`。
type Entry = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    FailureTriage,
);

/// 全块码表（一次性写全，下游片只消费、不得另存第二份文案）。
const TABLE: &[Entry] = &[
    // ── 七个布局码（具体规则依据 + 可执行处置动作，REQ-LAYOUT-02）──
    (
        "aggregate_root_is_git",
        "逻辑代码库根目录不能是 git 仓库",
        "布局规则「逻辑代码库根目录不能是 git 仓库」未满足：当前根目录下有 .git，会被当成单仓，成员仓就变成别人仓里的嵌套目录，聚合索引与冷启动都无法正确识别边界。",
        "另建一个空的普通目录作为逻辑代码库根（不要在某个 git 仓里，也不要在 git 仓里 mkdir），把成员仓移到它的直接下一层，然后重新登记。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_root_member_outside_root",
        "成员仓{member}不在逻辑代码库根的直接下一层",
        "布局规则「每个成员必须是逻辑代码库根的一层直接子目录（<LC根>/<成员>/）」未满足：当前成员路径多套了目录层级，或落在逻辑代码库根之外，聚合索引找不到它。",
        "把成员{member}移动到 <LC根>/<成员>/（只一层，不要再套目录），然后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_root_member_symlink_escape",
        "成员仓{member}经符号链接指向逻辑代码库根之外",
        "布局规则「成员必须是 LC 根内的真实目录，不能经符号链接指向根之外」未满足：该成员的链接目标在逻辑代码库根之外，索引会越过声明范围读写。",
        "用真实路径放置成员（不要把它 symlink 到别处），然后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_root_overlap",
        "这个聚合根已被另一个逻辑代码库占用{member}",
        "布局规则「一个聚合根只能归属一个逻辑代码库」未满足：当前根（或它的上下级目录）已是另一个逻辑代码库的根，两个 LC 会互相覆盖同一批成员。",
        "换一个未被占用的目录作为逻辑代码库根；要多个逻辑代码库就各自用互不重叠的根目录。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_root_nested_worktree",
        "成员仓{member}是嵌套 worktree",
        "布局规则「成员必须是正常检出，不能是嵌套 worktree」未满足：嵌套 worktree 的 git 元数据指向另一个工作区，索引快照无法稳定归属。",
        "成员改用正常检出（单独的 clone / 普通工作区），不要塞嵌套 worktree，然后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_index_layout_unsupported",
        "当前目录布局不被聚合索引支持",
        "布局规则「每个成员必须是逻辑代码库根的一层直接子目录（<LC根>/<成员>/）」未满足：索引只接受 LC 根下每个成员一层直挂、且成员是真实检出目录；当前 LC 根或成员的自定义相对路径无法安全遍历。",
        "让 LC 根与成员保持 <LC根>/<成员>/ 一层直挂，成员使用真实检出目录，然后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_index_member_coverage_failed",
        "成员仓里没有可索引的代码文件",
        "布局规则「每个成员至少要有一个可索引的源码文件」未满足：被纳入的成员{member}下找不到可索引的源码文件，索引会得到空结果。",
        "把真实源码放进成员仓（.js/.ts/.py/.go/.rs 等）；.html/.json/.md 不算源码；只放在 .worktrees/ 下的代码同样不算。放好后点「重试」。",
        FailureTriage::Environment,
    ),
    // ── 聚合初始化（durable 记录）链路 ──
    (
        "aggregate_machine_skills_failed",
        "这台机器的 Cadence skills 准备失败",
        "machine skills 是确定性本地步骤（不跑 provider），但这次没能准备成功，后续 provider 步骤因此没有启动。",
        "点「重试」重新准备 machine skills；连续两次失败时，检查 ~/.claude/skills 是否可写并把下面的诊断一并反馈。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_preflight_failed",
        "聚合根预检没有通过",
        "确定性预检（成员清单、根目录、排除集）发现了不满足的条件，provider 步骤因此没有启动。",
        "按下面的诊断先修掉预检发现的问题（权限、成员路径、排除集），再点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_aggregate_preflight_failed",
        "聚合根预检没有通过",
        "确定性预检（成员清单、根目录、排除集）发现了不满足的条件，provider 步骤因此没有启动。",
        "按下面的诊断先修掉预检发现的问题（权限、成员路径、排除集），再点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_policy_publication_failed",
        "聚合政策发布失败",
        "根配方的政策正文没能发布成不可变 artifact，readiness 因此不能冻结本次结果。",
        "点「重试」重新发布政策；持续失败时把下面的诊断一并反馈。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_pre_check_failed",
        "provider 的第一步（pre_check）没有完成",
        "provider turn 在 pre_check 步骤失败，后续两个 turn 与聚合索引构建都没有启动。",
        "先按下面对话诊断修掉 provider 侧问题（授权、命令、网络），再点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_rule_and_mcp_config_failed",
        "provider 的第二步（规则与 MCP 配置）没有完成",
        "provider turn 在规则/MCP 配置步骤失败，最后一步与聚合索引构建都没有启动。",
        "先按下面对话诊断修掉 provider 侧问题，再点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_openspec_and_examples_failed",
        "provider 的第三步（openspec 与示例文件）没有完成",
        "provider turn 在 openspec/examples 步骤失败，聚合索引构建没有启动。",
        "先按下面对话诊断修掉 provider 侧问题，再点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_provider_turn_failed",
        "provider 的一步没有完成",
        "provider turn 失败，后续步骤与索引构建没有启动。",
        "先按下面对话诊断修掉 provider 侧问题，再点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_initialization_store_failed",
        "聚合初始化的状态没能保存",
        "operation 记录的读取或写入失败，本次执行结果无法落成 durable 事实。",
        "点「重试」；连续两次仍失败时，检查 .aria 状态目录的权限与磁盘空间，并附上下面的诊断反馈。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_initialization_state_rejected",
        "本次状态流转被状态机拒绝",
        "durable 状态机认为这次流转不合法（并发的另一次执行可能已经推进过同一步）。",
        "刷新后按当前状态重新选择动作；仍失败时把下面的诊断一并反馈。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_initialization_trust_waiting",
        "正在等待 provider 的信任前置确认",
        "provider 尚未获得这台机器上的信任（登录/工作区信任），配方因此不能开始。",
        "按界面提示完成该 provider 的授权或信任确认，然后点「继续」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_initialization_cancelled",
        "本次聚合初始化已被取消",
        "操作在完成前被取消，已产生的中间事实按原样保留。",
        "需要时重新发起一次聚合初始化。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_initialization_interrupted",
        "服务在聚合初始化完成前中断",
        "进程在操作完成前退出，没有留下完整的终态事实。",
        "检查可能的部分修改后，重新提交一次聚合初始化。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_initialization_failed",
        "聚合初始化没有完成",
        "这次聚合初始化以失败结束，具体步骤信息见下方诊断。",
        "点「重试」；连续两次仍失败时，附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "root_recipe_failed",
        "根配方执行失败",
        "根配方上一条 operation 以失败结束，readiness 材料没有冻结。",
        "点「重试」重新执行根配方；持续失败时附诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    // ── 环境前置（片5 产出）──
    (
        "aggregate_root_not_writable",
        "聚合根目录不可写",
        "冷启动需要往聚合根下写政策与 receipt 文件，但当前目录不可写。",
        "修复聚合根目录的写权限（chmod / 换到可写目录）后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "skills_commands_missing",
        "四条命令的 skill 链接缺失",
        "~/.claude/skills 下缺少 pre-check / rule-config / mcp-configuration / project-rules-examples 的 SKILL.md 链接，配方里的四条命令无法解析。",
        "点「重试」重建 machine skills 链接；仍缺失时检查 ~/.claude 是否可写。",
        FailureTriage::Environment,
    ),
    (
        "codegraph_missing",
        "未检测到 codegraph",
        "聚合索引依赖 codegraph CLI，但当前 PATH 上找不到它。",
        "执行 npm i -g @colbymchenry/codegraph 安装（或把已有安装加入 PATH），装好后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "codegraph_version_mismatch",
        "codegraph 版本不在支持区间（>=1.6.0 <2.0.0）",
        "已安装的 codegraph 版本超出索引支持的区间，索引命令的输出协议可能不兼容。",
        "把 codegraph 更新（或回退）到 >=1.6.0 <2.0.0，然后点「重试」。",
        FailureTriage::Environment,
    ),
    // ── provider 能力/选择（片3/片4 产出）──
    (
        "aggregate_provider_capability_unverified",
        "{provider} 已安装，但本逻辑代码库还没核验它的能力",
        "能力证据是每个逻辑代码库一份；这台机器装了该 provider，但这份证据还没产生。",
        "点「核验 {provider}」跑一次能力探测，通过后重试本步。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_provider_capability_denied",
        "{provider} 实测不支持这一步",
        "能力探测已确认该 provider 在真实调用中拒绝了这一步（原因见诊断）。",
        "改用下一家 provider（界面会给出可选项），或按诊断修掉该 provider 的拒绝原因后重新核验。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_provider_capability_evidence_stale",
        "{provider} 的核验证据已过期",
        "该 provider 的能力证据与当前版本/配置不再匹配，按过期处理而不是直接信任。",
        "重新核验 {provider} 后重试本步。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_initialization_no_provider_available",
        "候选 provider 都不可用",
        "当前机器上没有任何一家 provider 能承担这一步（未安装或不可执行）。",
        "按下面的安装线索逐家安装一家 provider，装好后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_initialization_provider_needs_verification",
        "已安装的 provider 都还没有核验",
        "provider 已装好，但本逻辑代码库没有可用的能力证据，无法安全选择。",
        "先核验第一家已安装的 provider；通过后重试本步。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_initialization_provider_capability_denied",
        "已安装的 provider 都实测拒绝这一步",
        "候选 provider 中已安装的那些都在真实调用里拒绝了这一步，逐家原因见下方诊断。",
        "按诊断逐家修掉拒绝原因（授权/信任/版本），或安装另一家 provider 后重试。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_initialization_provider_health_degraded",
        "provider 健康状态异常（内部状态）",
        "provider 健康探测返回了异常的内部状态，产品无法据此判断可用性（这属于内部状态，不是你配置错）。",
        "先等该 provider 健康状态恢复后点「重试」；持续异常就换一家 provider，或附机器码与下面诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    // ── 审计预算（片6 产出）──
    (
        "aggregate_audit_budget_exceeded",
        "聚合根审计超出快照预算",
        "超预算不是某个文件特别大，而是纳入审计的条目总量过多（大项排行与成员仓见下方诊断）。",
        "两条出路：清理不再需要的大目录（如构建产物、依赖缓存），或把大目录写进 <LC根>/.git/info/exclude。不要改 .gitignore（它会进仓改变项目契约）。",
        FailureTriage::ProductDefect,
    ),
    // ── 聚合索引 durable warning（§13 已枚举的内层码）──
    (
        "aggregate_index_failed",
        "聚合索引构建失败",
        "索引命令以失败结束，具体原因见下方诊断。",
        "点「重试」重建聚合索引；连续两次仍失败时，附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_store_error",
        "聚合索引状态存储失败",
        "索引记录的读取/写入/校验失败，本次索引结果无法落成 durable 事实。",
        "点「重试」；持续失败时检查 .aria 状态目录的权限与磁盘空间，并附诊断反馈。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_lock_error",
        "聚合索引的锁没能拿到",
        "另一个索引操作持有锁，或锁文件不可访问，本次构建没有开始。",
        "等当前构建结束后再点「重试」；持续失败时检查状态目录里锁文件的可访问性。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_index_manifest_missing",
        "聚合索引缺少成员清单",
        "索引需要当前逻辑代码库的成员清单，但清单不存在或未刷新。",
        "先完成（或重新进入）逻辑代码库的成员登记，再点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_index_member_invalid",
        "某个成员的登记事实不一致",
        "成员记录与主检出的权威身份对不上，索引无法确定要读哪一份代码。",
        "刷新成员登记事实后点「重试」；仍失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_invalid_revision",
        "成员仓的 revision 不可用",
        "取到的成员 revision 不是有效的提交标识，索引无法固定输入快照。",
        "确认成员仓有有效的 HEAD 后重试；已有提交仍格式异常时附诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_git_runtime",
        "Git 运行时不可用",
        "索引过程中的 Git 调用环境创建失败（非用户输入问题）。",
        "点「重试」；持续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_git_runner_panic",
        "Git 调用线程异常退出",
        "索引的 Git 子任务 panic 了，结果不可信。",
        "点「重试」；连续两次仍失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_git_timeout",
        "Git 操作超时",
        "索引等待 Git 命令超过了允许的时限，本次构建中止。",
        "等 Git 操作（如大仓 checkout/status）结束后点「重试」。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_index_git_cancelled",
        "Git 操作被取消",
        "索引用到的 Git 调用被取消，本次构建中止。",
        "需要时重新发起一次聚合索引重建。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_index_git_failed",
        "Git 命令返回失败",
        "索引依赖的一条 Git 命令以非零状态结束，具体输出见下方诊断。",
        "按诊断修掉 Git 侧问题后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_index_git_missing",
        "未检测到可用的 Git",
        "聚合索引需要 Git 来固定成员快照，但当前环境找不到 git 可执行文件。",
        "安装 Git 或修复 PATH 后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_index_git_io",
        "Git 调用发生 IO 错误",
        "启动 Git 或读取它的输出时发生 IO 错误。",
        "按诊断确认权限/磁盘原因后点「重试」；原因不明时直接重试并附诊断反馈。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_already_exists",
        "聚合索引记录已存在",
        "本代索引 id 已被占用，重复创建被拒绝（不会删除既有记录）。",
        "点「重试」重新规划一次索引；持续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_identity_mismatch",
        "聚合索引记录的身份不一致",
        "记录里的项目/逻辑代码库身份与当前请求不一致，继续写入会污染状态。",
        "停止手动改写状态数据，附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_active_ambiguous",
        "存在多个活跃聚合索引",
        "同一逻辑代码库上出现多于一条活跃索引记录，内部状态异常。",
        "保留现有记录并附机器码与诊断提交问题（不要手动删除记录）。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_not_found",
        "找不到聚合索引记录",
        "本次操作引用的索引记录不存在（可能已被并发替换）。",
        "刷新后重试；仍找不到记录时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_invalid_record_path",
        "索引状态目录里的记录路径非法",
        "索引记录的文件路径超出了状态目录允许的范围，读写被拒绝。",
        "不要手动删除随机文件；附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_project_mismatch",
        "索引记录的项目身份不一致",
        "记录上冻结的项目身份与当前项目不一致，写入会跨项目污染。",
        "附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_member_drifted",
        "成员工作区在构建期间发生漂移",
        "构建期间成员的 HEAD/工作区发生变化，索引快照不再稳定。",
        "停止本轮改动，等成员工作区稳定后点「重试」重建。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_index_exclusion_failed",
        "索引排除契约没有满足",
        "索引要求某些目录（如 .git、assets）必须被排除，但当前配置不满足。",
        "点「重试」；持续失败时附机器码与诊断提交问题（不要放宽排除集来绕过）。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_io",
        "读取索引配置时发生 IO 错误",
        "扫描/读取聚合根配置时发生 IO 错误。",
        "按诊断确认根目录读取权限/磁盘原因后点「重试」。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_json",
        "索引配置序列化失败",
        "索引配置的 JSON 编码失败，说明配置内容不满足协议。",
        "点「重试」；持续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_io",
        "codegraph CLI 启动发生 IO 错误",
        "启动 codegraph 可执行文件时发生 IO 错误。",
        "按诊断确认路径/权限原因后点「重试」；原因不明时直接重试并附诊断反馈。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_runner_runtime",
        "codegraph 运行环境创建失败",
        "为 codegraph 准备运行环境时失败（内部状态）。",
        "点「重试」；持续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_runner_panic",
        "codegraph 运行线程异常退出",
        "codegraph 子任务 panic 了，本次结果不可信。",
        "点「重试」；连续两次仍失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_timeout",
        "codegraph 命令超时",
        "codegraph 命令超过允许时限，本次索引中止。",
        "等系统负载下降后点「重试」。",
        FailureTriage::RetryOnly,
    ),
    (
        "codegraph_cancelled",
        "codegraph 命令被取消",
        "本次 codegraph 调用被取消。",
        "需要时重新发起一次聚合索引重建。",
        FailureTriage::RetryOnly,
    ),
    (
        "codegraph_version_failed",
        "codegraph 版本探测失败",
        "读取 codegraph 版本的命令失败，无法确认它是否在支持区间。",
        "按诊断修复 codegraph 安装后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "codegraph_init_failed",
        "codegraph 索引初始化失败",
        "codegraph init 以失败结束，输出见下方诊断。",
        "点「重试」；持续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_sync_failed",
        "codegraph 索引同步失败",
        "codegraph sync 以失败结束；部分下游状态可能需要重新初始化。",
        "按诊断修复后点「重试」；持续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_files_failed",
        "codegraph 文件清单命令失败",
        "读取索引文件清单的命令以失败结束，输出见下方诊断。",
        "点「重试」；持续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_files_invalid_json",
        "codegraph 文件清单输出不是合法 JSON",
        "上游 CLI 的输出协议与预期不符（版本/实现问题）。",
        "附机器码、codegraph 版本与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_files_output_truncated",
        "codegraph 文件清单超出抓取上限",
        "索引文件太多，清单输出超过抓取上限被截断，得到的清单不完整（不是上游协议异常）。",
        "缩小聚合根规模后重新发起聚合索引重建；持续出现时附机器码与诊断提交问题。",
        FailureTriage::Environment,
    ),
    (
        "codegraph_sdk_not_found",
        "codegraph SDK 未找到",
        "找不到 codegraph 的 npm-sdk.js（未安装 codegraph，或安装前缀未知），SDK 成员覆盖验收无法执行。",
        "手动安装：npm install -g @colbymchenry/codegraph；已安装时确认 npm 全局前缀后重试。",
        FailureTriage::Environment,
    ),
    (
        "codegraph_node_unavailable",
        "缺少可用的 Node.js",
        "运行 codegraph SDK 覆盖验收需要 Node.js ≥ 22.5（或 codegraph 平台包自带的 Node），当前两者都不可用。",
        "安装 Node.js ≥ 22.5，或重新安装 codegraph（npm install -g @colbymchenry/codegraph）后重试。",
        FailureTriage::Environment,
    ),
    (
        "codegraph_coverage_failed",
        "codegraph 成员覆盖验收失败",
        "SDK 覆盖验收 helper 执行失败（打开索引库或查询成员覆盖时出错，诊断见下方）。",
        "点「重试」；持续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_sdk_incompatible",
        "codegraph SDK 版本不兼容",
        "已安装 codegraph 的 SDK 不提供验收所需方法（CodeGraph.open / hasFilesUnder），版本与预期不符。",
        "把 codegraph 升级到受支持区间（>=1.6.0, <2.0.0）后重试。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_query_failed",
        "codegraph 查询命令失败",
        "索引的负向验收查询以失败结束。",
        "点「重试」；持续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "codegraph_query_invalid_json",
        "codegraph 查询输出不是合法 JSON",
        "上游 CLI 的查询输出协议与预期不符。",
        "附机器码、codegraph 版本与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "aggregate_index_membership_revision_mismatch",
        "成员版本与这次索引请求不一致",
        "请求绑定的成员版本已经过期（登记事实被刷新过）。",
        "刷新当前成员版本后重新发起聚合索引重建。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_index_command_revision_mismatch",
        "命令绑定的版本与当前版本不一致",
        "该命令创建时的成员版本已过期，不能复用。",
        "刷新后用当前动作入口重新发起，不要复用冲突的旧命令。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_index_rebuild_in_progress",
        "已经有一次聚合索引重建在进行中",
        "同一逻辑代码库上同时只允许一次重建。",
        "等这次构建结束后刷新页面再操作，不要并发重复发起。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_index_command_replayed_failure",
        "该命令此前已失败，本次为幂等重放",
        "同一命令键的既有事实是失败，重放不会重新构建。",
        "查看上一次的失败诊断，用既有的「重试」入口重新发起一次新构建。",
        FailureTriage::RetryOnly,
    ),
    (
        "aggregate_index_unavailable",
        "聚合索引服务当前不可用",
        "索引重建的工作线程异常退出或服务未就绪，没有可用的构建结果。",
        "点「重试」；连续两次仍失败时，附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    // ── 跨仓证据检索（evidence_index_*）：索引不可用 ⇒ 重建索引后重试 ──
    (
        "evidence_index_unavailable",
        "聚合索引不可用，跨仓证据检索无法执行",
        "这不是「没有找到证据」：跨仓证据检索要读聚合索引，当前索引不可用（尚未建立，或记录在册但磁盘上的 codegraph 库目录已不在），检索根本没有执行，所以这次没有任何命中结果。",
        "在逻辑代码库管理面板的「聚合索引」处点「重建索引」重建聚合索引；重建完成后重新发起跨仓证据检索。",
        FailureTriage::Environment,
    ),
    // ── 规则/政策等待（投影层既有码）──
    (
        "aggregate_policy_artifact_missing",
        "权威引用冻结的政策正文已不在场",
        "引用仍然指向某个 policy，但对应的 artifact 文件消失了，readiness 不能视为就绪。",
        "点「重新生成政策」用当前源重新发布 policy。",
        FailureTriage::ProductDefect,
    ),
    (
        "policy_reference_digest_drift",
        "权威引用的摘要与当前政策正文不一致",
        "引用冻结的 digest 和当前 artifact 的 digest 不同，说明政策被并发替换过。",
        "点「重新生成政策」用当前源重新生成引用。",
        FailureTriage::ProductDefect,
    ),
    (
        "policy_digest_drift",
        "receipt 冻结的政策摘要与当前正文不一致",
        "上次配方冻结的 policy digest 已过期，readiness 材料与当前权威正文不匹配。",
        "点「重新生成政策」重新发布并冻结新的 receipt。",
        FailureTriage::ProductDefect,
    ),
    (
        "rule_digest_drift",
        "receipt 冻结的规则摘要与当前根规则不一致",
        "聚合根的规则文件在配方之后被改动，receipt 不再代表当前事实。",
        "点「重新生成政策」重新生成并冻结 receipt。",
        FailureTriage::ProductDefect,
    ),
    (
        "root_recipe_operation_missing",
        "这个逻辑代码库还没跑过根配方",
        "政策已就绪，但当前逻辑代码库下没有任何根配方 operation，readiness 材料不会自己出现。",
        "点「准备」发起一次根配方执行。",
        FailureTriage::Environment,
    ),
    (
        "root_recipe_cancelled",
        "根配方在就绪材料冻结前被取消",
        "上次根配方被取消，readiness 材料没有落全。",
        "点「继续」重新推进，或重新发起一次根配方。",
        FailureTriage::RetryOnly,
    ),
    (
        "root_receipt_missing",
        "根配方完成但没有留下最终 receipt",
        "operation 报成功，却没有冻结的 root receipt，readiness 不能视为就绪。",
        "点「重试」重新执行根配方；持续失败时附诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
    (
        "root_receipt_authority_drift",
        "receipt 冻结的聚合根与当前权威根不一致",
        "receipt 记录的 canonical root 与解析出的权威根不同，说明根目录被换过。",
        "点「核验」重新核对权威根；确认无误后重新生成政策。",
        FailureTriage::ProductDefect,
    ),
    (
        "root_rule_missing",
        "聚合根上缺少根规则文件",
        "根配方依赖的根规则文件（AGENTS.md）不在 canonical root 下。",
        "补齐聚合根上的根规则文件后点「重试」。",
        FailureTriage::Environment,
    ),
    (
        "aggregate_policy_bootstrap_placeholder",
        "当前聚合政策仍是已知自举桩正文",
        "readiness 读到的政策还是自举桩（未真实生成），不能冒充根配方权威正文。",
        "重新发起一次聚合初始化（携带新的 idempotency_key）跑真实根配方；通用「重试」不会重跑根配方。",
        FailureTriage::Environment,
    ),
    (
        "identity_migration_failed",
        "身份迁移失败",
        "既有单仓身份迁移到逻辑代码库身份的 journal 是失败态。",
        "点「去修复」按诊断修复身份映射后重试。",
        FailureTriage::ProductDefect,
    ),
    (
        "bootstrap_waiting_for_human",
        "这一步在等待人工处理",
        "该步骤没有失败，但需要你显式选择动作才能继续。",
        "按界面上给出的动作（准备/继续/重试/核验）选择一项。",
        FailureTriage::Environment,
    ),
    // ── 路由归属（REQ-ROUTE-01：单仓入口删除后的存量记录读取 / 载体判定
    //     无归属 / 创建提交缺归属字段；码表是用户可见失败面文案唯一来源）──
    (
        "issue_not_bound_to_logical_codebase",
        "这个 issue 没有归属逻辑代码库",
        "单仓 issue 入口已删除，每条 issue 都必须归属一个逻辑代码库；这条 issue 没有归属信息（单仓时代的存量记录，或提交时缺少归属字段），产品不会替你猜它属于哪个物理仓。",
        "给这条 issue 选择一个逻辑代码库归属（存量 issue 在编辑里补选），保存后重试；还没有可用的逻辑代码库时，先把成员登记进逻辑代码库再回来创建。",
        FailureTriage::Environment,
    ),
    // ── LC 知识库模块（Task 7：HTTP 面稳定码；明细见 details 的
    //     missing / violations / paths 字段）──
    (
        "knowledge_base_requires_aggregate_initialization",
        "聚合初始化未完成，知识库暂不可用",
        "该逻辑代码库的聚合初始化（冷启动投影的「规则/政策」步）还没有完成；在它完成之前，知识库的一切操作（建立、更新、文件与输入读写）都不可用。",
        "先到逻辑代码库管理里完成聚合初始化，再回来使用知识库；若此前已完成、因规则摘要漂移被重新判为未完成，先在该流程执行「重新生成政策」（Republish）收敛后再试。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_skills_unavailable",
        "被选 provider 缺知识库技能",
        "被选 provider 对应的 skill 层缺少 7 个 knowledge-base 技能（缺哪些见下方明细），通常是 Cadence-skills 未安装或版本过旧。",
        "到 Provider 管理里安装或更新 Cadence-skills 后重试。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_inputs_incomplete",
        "初始化输入还没填完",
        "user-input/ 下按六领域状态判定仍有必填文件缺失（缺哪些见下方明细）；「不适用」的领域不要求，其余一律必填。",
        "把缺失的输入文件补齐（或把对应领域标成「不适用」并写明原因）后重试。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_manifest_missing",
        "知识库清单不存在",
        "cadence/knowledge-base/manifest.yaml 不存在，知识库尚未建立或清单被移走。",
        "先完成知识库建立（bootstrap），再执行更新操作。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_manifest_version_unsupported",
        "知识库清单版本不受支持",
        "manifest.yaml 的 schema_version 不是 \"4.0\"（实际取值见下方明细），当前技能契约只支持 Schema 4.0。",
        "用受支持版本的技能重建知识库，或把清单恢复为 Schema 4.0 形态。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_not_initialized",
        "知识库初始化未完成",
        "manifest.yaml 的初始化状态不是 complete/passed，或必需阶段缺失、乱序等结构不变量不满足——按未完成拒绝，不指望下游兜底。",
        "回到知识库建立流程把初始化跑完（global-validation 通过）后再更新。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_manifest_invalid",
        "知识库清单内容非法",
        "manifest.yaml 在场但读不出合法内容，按「版本未知」降级后被前置检查拒绝。",
        "修复或重建 manifest.yaml（Schema 4.0 形态）后重试。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_change_package_invalid",
        "变更包没过平台门禁",
        "变更包的机械门禁有违规（逐条见下方明细：哪份文档、哪个字段、什么原因），门禁不过不创建更新操作。",
        "按明细逐条修复变更包内容后重新提交。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_change_package_path_invalid",
        "变更包路径或标识非法",
        "变更包路径/标识必须是知识库内 user-input/updates/ 下的相对目录，只允许 [A-Za-z0-9._-]，不含路径分隔符或「..」。",
        "改用合法的变更包路径或标识后重试。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_change_package_missing_document",
        "变更包缺必备文档",
        "变更包必备的五份文档（change-summary / code-change / database-change / configuration-change / verification）有缺失或内容为空（哪份见下方明细）。",
        "把缺失的文档按模板补齐后重新提交。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_change_package_field_empty",
        "变更包必填字段为空",
        "变更包文档里的必填字段为空（哪份文档、哪个字段见下方明细）。",
        "按模板把空字段填好后重新提交。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_change_package_reason_missing",
        "变更包缺少变更原因",
        "变更包没有写清楚这次变更的原因，平台门禁要求每个变更包都有明确动机。",
        "在变更包里补充变更原因后重新提交。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_change_package_domain_matrix_invalid",
        "领域变更矩阵填写非法",
        "change-summary.md 的领域变更矩阵取值不合法（哪个格子、什么问题见下方明细）。",
        "按矩阵模板修正领域取值后重新提交。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_change_package_sensitive_content",
        "变更包疑似包含敏感内容",
        "变更包里扫描到密钥/令牌类敏感形态（位置见下方明细，明细不回显命中值原文）。",
        "把敏感内容移出变更包（改用环境变量或密钥管理）后重新提交。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_change_package_commit_unresolvable",
        "变更包引用的提交无法解析",
        "变更包里的 Merge Request 地址或编号在任一成员仓都解析不出提交（全部成员仓尝试失败，含成员列表为空的情况）。",
        "确认提交编号已推送到成员仓，或修正编号后重新提交。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_change_package_scope_mismatch",
        "变更范围与清单口径不一致",
        "变更包声明的领域范围与知识库 manifest 的 scope 口径不一致（哪个领域见下方明细）。",
        "让变更包的领域声明与知识库 scope 一致后重新提交。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_file_conflict",
        "文件已被别人改过，请先刷新",
        "提交的 base_digest 与服务端当前内容摘要不一致——写入窗口内文件内容发生了变化，乐观并发拒绝覆盖。",
        "重新读取该文件拿到最新内容与摘要后再编辑提交。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_operation_running",
        "知识库操作运行中，稍后再试",
        "这条逻辑代码库有知识库 operation 正在跑（建立/更新互斥），运行期文件写入与状态变更被暂停。",
        "等当前操作结束后重试，或先取消在跑的操作。",
        FailureTriage::RetryOnly,
    ),
    (
        "knowledge_base_operation_not_found",
        "知识库操作不存在",
        "按编号找不到这条逻辑代码库的知识库 operation（从未创建，或编号不属于这个逻辑代码库）。",
        "从知识库面板重新发起或刷新操作列表。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_write_scope_violation",
        "provider 越界写入，操作已判失败",
        "provider turn 前后的快照差分发现允许面之外的新增/修改/删除（越界路径见下方明细）；允许面只有 cadence/knowledge-base、cadence/project-rules 与根 CLAUDE.md/AGENTS.md。",
        "检查越界改动并恢复工作区后，重新发起知识库操作。",
        FailureTriage::Environment,
    ),
    (
        "other_knowledge_base_running",
        "已有知识库操作在跑，不能重复发起",
        "同一条逻辑代码库的知识库建立与更新互斥（单飞），已有 operation 在跑时拒绝新启（在跑操作编号见下方明细）。",
        "等在跑的操作结束，或先到面板取消它。",
        FailureTriage::RetryOnly,
    ),
    (
        "knowledge_base_provider_capability_denied",
        "provider 拒绝了知识库写入分格",
        "候选 provider 在真实调用里拒绝了知识库所需的 CodingTargetWrite 分格（被拒 provider 见下方明细）。",
        "按诊断修掉拒绝原因（授权/信任/版本），或换一家 provider 后重试。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_file_path_escapes",
        "路径越出知识库根",
        "文件路径是绝对路径、含「..」或经符号链接逃出知识库根，读取/写入授权只覆盖知识库根内部。",
        "改用知识库内的相对路径重试。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_file_not_found",
        "文件不存在",
        "目标文件不在知识库内或不可读（知识库尚未建立时根目录不存在也落此处）。",
        "确认路径无误；尚未建立知识库的先完成建立。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_file_not_editable",
        "该路径不在可编辑白名单内",
        "写入白名单只覆盖知识库内的 *.md 与 user-input/** 下的 *.sql，机器管理文件（如 manifest.yaml）不可经文件面改写。",
        "只编辑白名单内的文档文件。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_file_too_large",
        "文件超过大小上限",
        "单文件读取/写入上限为 1 MiB，目标文件超限。",
        "拆分文档或精简内容后重试。",
        FailureTriage::Environment,
    ),
    (
        "invalid_knowledge_base_request",
        "知识库请求参数不合法",
        "请求体缺了必填字段或取值非法（kind 只能是 bootstrap/update；update 必须带 package_path；idempotency_key 不能为空）。",
        "按端点契约修正请求体后重试。",
        FailureTriage::Environment,
    ),
    // ── LC 知识库输入确认面（收尾 A，REQ-KB-14/15：两段式分叉与确认端点
    //     的两个新稳定码；明细见 details 的 unconfirmed / path 字段）──
    (
        "knowledge_base_user_input_not_confirmed",
        "初始化输入还有未确认的文件",
        "user-input/ 下的初始化输入已准备就绪，但仍有文件未确认（哪些未确认见下方明细）；全部确认之前，知识库建立不会开始。",
        "到知识库输入确认区逐份核对并确认全部输入文件（内容有改动就先改再确认），然后重新发起知识库初始化。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_user_input_confirmation_stale",
        "输入内容已变化，这次确认已过期",
        "确认请求携带的摘要与服务端当前文件内容不一致——文件在确认前被改动过，本次确认被整批原子拒绝，没有落下任何确认记录。",
        "刷新读取该文件的最新内容与摘要后重新确认。",
        FailureTriage::Environment,
    ),
    // ── LC 知识库执行期失败码（operation 落盘记录的顶层 reason_code；
    //     卡片/投影经 failure_copy 同码表渲染，不登记就落「本次操作未完成」
    //     中性兜底，真实原因会被吞掉）──
    (
        "knowledge_base_provider_turn_failed",
        "知识库操作执行失败",
        "provider turn 以失败结束（provider 的输出摘要见下方诊断；超时类失败可直接重试）。",
        "按诊断里的 provider 输出摘要处理问题后重新发起知识库操作。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_provider_interaction_required",
        "provider 在知识库操作中发起了提问",
        "本次知识库操作是无人值守执行（没有应答通道）：provider 在执行中请求授权或向用户提问，本次执行因此中止——它问了什么见下方诊断。",
        "按诊断里 provider 的提问补齐输入文件或消解该问题；无法消解时改用另一家 provider 重新发起。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_cancelled",
        "知识库操作已取消",
        "操作在完成前被取消，provider 已确认停止，已产生的改动按原样保留在聚合根工作树里。",
        "确认聚合根工作树状态后按需重新发起。",
        FailureTriage::RetryOnly,
    ),
    (
        "knowledge_base_cancel_audit_indeterminate",
        "操作已取消，且无法确认 provider 已停止写入",
        "取消返回时没能确认 provider 进程已停止，已写入范围只算下界（provider 可能仍在写）；审计按「不确定」留痕，不当作干净取消。",
        "人工检查聚合根工作树（git status / git diff）确认没有越界改动后，再决定是否重新发起。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_invalid_input",
        "知识库操作输入不合法",
        "操作输入不满足知识库执行契约（具体原因见下方诊断）。",
        "按诊断修正输入后重新发起知识库操作。",
        FailureTriage::Environment,
    ),
    (
        "knowledge_base_internal_error",
        "知识库操作内部错误",
        "知识库操作在服务端内部环节失败（不是输入问题；上下文见下方诊断）。",
        "查看服务端日志定位原因后重新发起；连续失败时附机器码与诊断提交问题。",
        FailureTriage::ProductDefect,
    ),
];

/// 家族模板：码表未收但语义可判定的族（仍给出完整三段，不落到未知兜底）。
const FAMILY_ENTRIES: &[(&str, Entry)] = &[
    (
        "_timeout",
        (
            "&timeout",
            "操作超时",
            "这一步等待超过了允许的时限，本次执行中止。",
            "等系统负载下降或前置操作结束后点「重试」。",
            FailureTriage::RetryOnly,
        ),
    ),
    (
        "_non_zero_exit",
        (
            "&non_zero_exit",
            "外部命令以非零状态退出",
            "依赖的外部命令失败，输出见下方诊断。",
            "按诊断修掉外部命令的问题后点「重试」。",
            FailureTriage::Environment,
        ),
    ),
    (
        "_cancelled",
        (
            "&cancelled",
            "操作被取消",
            "本次执行被取消，已产生的中间事实按原样保留。",
            "需要时重新发起一次。",
            FailureTriage::RetryOnly,
        ),
    ),
];

/// 内部码 → 对外稳定码；未命中别名即原码。
pub fn canonical_key(code: &str) -> &str {
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == code)
        .map(|(_, canonical)| *canonical)
        .unwrap_or(code)
}

fn table_entry(key: &str) -> Option<Entry> {
    TABLE
        .iter()
        .find(|entry| entry.0 == key)
        .copied()
        .or_else(|| {
            FAMILY_ENTRIES
                .iter()
                .find(|(suffix, _)| key.ends_with(suffix))
                .map(|(_, entry)| *entry)
        })
}
