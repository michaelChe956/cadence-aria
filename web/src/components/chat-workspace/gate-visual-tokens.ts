// F-50 视觉 v2（f50-ui-visual-spec-v2.md）：门卡（chat 流 GatePromptEntry）与
// 抽屉门禁条目（CockpitInbox）共用的视觉常量——色彩 token 表 §1 / 门卡结构 §2 /
// 抽屉规则 §3 / 图标清单 §4。只做视觉层：信息架构/文案/动作语义不动（F50Ui 已上线）。
//
// 主题口径：当前 App 为 light 单主题（styles.css :root 是唯一色源，无 dark 开关），
// 类名取规格 Light 列；Dark 列（slate-900 卡底/slate-100 正文/text-amber-400 chip 等）
// 以注释留档，全站 dark 主题落地时在此一处对齐。
import { FileText, Gauge, RefreshCw, Terminal, Wrench, Zap } from "lucide-react";

/** §1 门卡=中性底（dark: bg-slate-900），语义只上左边框/标签。 */
export const GATE_CARD_CLASS =
  "rounded-xl border border-slate-200 border-l-4 border-l-amber-500/60 bg-white px-5 py-4 shadow-lg transition-colors duration-200";

/** §2 标题行 chip：琥珀只出现在标签（dark: bg-amber-950/40 text-amber-400）。 */
export const GATE_CHIP_CLASS =
  "ml-2 inline-flex shrink-0 items-center rounded-full bg-amber-100 px-2.5 py-0.5 text-[11px] font-semibold text-amber-700";

/** §2 标题行正文主色（dark: text-slate-100）。 */
export const GATE_TITLE_CLASS = "min-w-0 truncate text-base font-semibold text-slate-900";

/** §2 原因行/正文次色（dark: text-slate-400）。 */
export const GATE_SECONDARY_TEXT_CLASS = "text-slate-600";

/** §2 元数据行（dark: text-slate-500）。 */
export const GATE_META_TEXT_CLASS = "text-slate-500";

/** §2 分节线。 */
export const GATE_DIVIDER_CLASS = "mt-3 border-t border-slate-200 pt-3";

/** §1 嵌套块底：层级每降一档换底（dark: bg-slate-800/60）。 */
export const GATE_NESTED_BLOCK_CLASS =
  "rounded-md border border-slate-200 bg-gray-50 px-3 py-2";


/** §2 主操作（确认）：实心 emerald（两主题同值）。 */
export const BTN_PRIMARY_CLASS =
  "inline-flex min-h-9 items-center justify-center gap-1 rounded-md bg-emerald-600 px-4 text-xs font-semibold text-white transition-colors duration-200 hover:bg-emerald-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-emerald-600";

/** §2 次操作（查看产物/提交反馈/重试）：描边（dark: border-slate-600 text-slate-200）。 */
export const BTN_SECONDARY_CLASS =
  "inline-flex min-h-9 items-center justify-center gap-1 rounded-md border border-slate-300 bg-transparent px-3 text-xs font-semibold text-slate-700 transition-colors duration-200 hover:border-slate-400 hover:bg-slate-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400";

/** §2 Ghost（终止）：无底描边透明（dark: text-red-400 hover:bg-red-950/40）。 */
export const BTN_GHOST_CLASS =
  "inline-flex min-h-9 items-center justify-center gap-1 rounded-md border border-transparent bg-transparent px-3 text-xs font-semibold text-red-600 transition-colors duration-200 hover:bg-red-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-400";

/** §2 卡内告警子卡：左线式，不再琥珀底叠琥珀底（告警=信息，门卡=决策）。 */
export const GATE_ALERT_SUBCARD_CLASS = "border-l-4 border-l-amber-500/60 bg-transparent px-3 py-2";

/** §2 反馈输入行：input（dark: bg-slate-800 border-slate-700）。 */
export const GATE_INPUT_CLASS =
  "min-h-9 min-w-0 flex-1 rounded-md border border-slate-300 bg-gray-50 px-3 text-sm text-slate-900 placeholder:text-slate-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400";

/** §4 details/summary 折叠：原生 ▶ marker 隐藏，chevron-right 随 open 旋转。 */
export const DISCLOSURE_SUMMARY_CLASS =
  "cursor-pointer list-none [&::-webkit-details-marker]:hidden";
export const DISCLOSURE_CHEVRON_CLASS =
  "h-4 w-4 shrink-0 text-slate-500 transition-transform duration-200 group-open:rotate-90 motion-reduce:transition-none";

/**
 * §4 emoji 图标退役：执行事件按 kind 映射 Lucide SVG（16px stroke-current，色随
 * 语义）——✨ provider started→zap、🟣 token usage→gauge、⚡ command/output→
 * terminal、artifact→file-text；prompt 判定优先（content_ref/title）。
 */
export function executionEventIcon(kind: string | undefined, isProviderPrompt: boolean) {
  if (isProviderPrompt) {
    return FileText;
  }
  switch (kind) {
    case "provider":
      return Zap;
    case "usage":
      return Gauge;
    case "command":
    case "output":
      return Terminal;
    case "artifact":
      return FileText;
    case "turn":
      return RefreshCw;
    default:
      return Wrench;
  }
}
