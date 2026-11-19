export interface ChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: string[];
}

export const CURRENT_VERSION = "0.1.9";

// 发布升版本时请裁剪数组，仅保留最新 4 条。
export const CHANGELOG: ChangelogEntry[] = [
  {
    version: "0.1.9",
    date: "2026-11-19",
    title: "重建聚合索引：查询耗时不再挤占源码复核时间",
    highlights: [
      "CodeGraph 返回覆盖名单后，Aria 才开始计算盘检的 2 秒时限。只有确认无源码的成员才跳过；无法完成检查时仍会提示原因。",
      "重建失败后，卡片会重新读取本次索引状态，不再停留在上一次的诊断。",
      "日志包可查看本次重建的覆盖数、分段耗时和逐仓盘检结果。Pi 会话识别支持更多原生存放方式，宿主批准命令默认使用 npx。",
    ],
  },
  {
    version: "0.1.8",
    date: "2026-11-19",
    title: "配置仓不再拖垮整库索引 · 生成 Story 前自动更新索引",
    highlights: [
      "只有配置和文档、没有源码的成员仓不再让聚合索引构建失败：确认无源码即跳过并点名提示，该仓仍是项目成员、仍可作为 Issue 目标仓",
      "生成或重新生成 Story 前自动确认共享索引与本地代码一致：有变化或无法确认未变（工作区有未提交修改）时先增量同步并重新验收，失败停在 AI 启动之前并给出稳定码",
      "纯配置仓作为 Issue 目标时不再发起 CodeGraph 证据查询，provider 直接在该仓工作区读写配置文件",
      "聚合索引失败提示改为「问题 / 原因 / 怎么做 + 机器码」三段并点名成员，冷启动卡片、HTTP 错误与只读诊断三处同码同文案",
    ],
  },
  {
    version: "0.1.7",
    date: "2026-11-16",
    title: "LC 初始化失败不再误放行 · 配置与聚合索引分离",
    highlights: [
      "预检脚本明确失败时，即使已经修改文件也不再判为通过；保留审计证据和真实失败诊断，后续步骤停止推进",
      "LC 规则与 MCP 配置阶段不再重复构建或等待 CodeGraph 索引；初始化成功后仍由平台自动执行聚合索引构建",
      "幂等执行允许零文件变化，不再要求为了通过初始化而制造文件变更；配置完成不等于索引已就绪",
      "升级前请先更新 Cadence-skills（包含 #117）：兼容 OpenSpec 2.0 的 Pi/Kimi 共享技能布局，并支持延后索引参数；无需删除已有配置或索引",
      "运行信息、npm 包和产品内版本号统一为 0.1.7，不再沿用历史内嵌版本号",
    ],
  },
  {
    version: "0.1.6",
    date: "2026-11-14",
    title: "点「安装」真的装得上 · 更新装回你正在用的那份 · 超大仓库的聚合索引不再误报 · 页面不再一直轮询",
    highlights: [
      "本机没装 provider 或 codegraph 时，点【安装】现在真的装得上并能用起来：安装目录会被纳入平台的命令可见范围（此前装到了平台看不见的位置，界面一直显示「未安装」，重复点击也没效果）",
      "Kimi Code 走官方脚本安装时也能被识别并探测到版本（此前脚本只改 shell 配置文件，正在运行的服务看不到它，而且不给出任何提示）",
      "新增「暴露到终端 PATH」：点一下就在你终端 PATH 上的目录里建一个软链，之后在终端里直接敲命令名也能用（只在你点它时才动作，不改你的 shell 配置文件）",
      "更新会装回你实际在用的那一份：当机器上同时存在多份同一个 CLI 时，更新此前可能写进你没在跑的那份，表现为「更新了却没变化」；现在更新后版本没变会明确报出来",
      "大型聚合根的索引验收不再把「输出超限」误报成「JSON 解析失败」：改为逐个成员目录判定，索引很大也能通过；确实超限时给出专用提示并保留上一份可用索引",
      "页面不再一直调用同一批接口：没有进行中任务时轮询间隔自动退到 60 秒、页面不可见时暂停、重新可见时立刻补一次；心跳不再触发整轮刷新，也不再对普通单仓发起必然失败的请求",
      "新增只读诊断接口 GET /api/providers/diagnostics：一次请求即可看到每个对象装到哪个目录、是否在你的终端 PATH 上、codegraph 平台包是否匹配、索引状态与最近失败原因",
    ],
  },
];

export function recentEntries(currentVersion: string, limit = 4): ChangelogEntry[] {
  const currentVersionIndex = CHANGELOG.findIndex((item) => item.version === currentVersion);
  if (currentVersionIndex === -1) {
    return [];
  }

  return CHANGELOG.slice(currentVersionIndex, currentVersionIndex + limit);
}
