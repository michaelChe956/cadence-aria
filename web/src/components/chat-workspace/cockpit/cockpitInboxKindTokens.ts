// 从 CockpitInbox.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 收件箱条目的 kind → 图标 / 视觉类名映射（主区与硬错误条共用）。
import {
  AlertTriangle,
  CircleAlert,
  CircleCheck,
  ClipboardList,
  ListChecks,
  Play,
  RotateCcw,
} from "lucide-react";
import { GATE_CARD_CLASS } from "../gate-visual-tokens";

export const KIND_GLYPH = {
  gate: ClipboardList,
  stopped: CircleAlert,
  hard_error: AlertTriangle,
  choice: ListChecks,
  info: CircleCheck,
  c1_recovery: RotateCcw,
  sc_failed: CircleAlert,
  lc_bootstrap: Play,
  // Main 授权（2026-10-12，最小构建解锁）：D7.1 恢复卡尚未真正渲染，
  // 先补中性机械条目（glyph 复用 RotateCcw，class 沿用既有卡片风格）；
  // 后续真正渲染恢复卡时复用或改。
  lc_resume_recovery: RotateCcw,
  // P4-d/P4-e（REQ-UI37-19）：durable 待处理事项（coding_attention 来源），
  // 与页面内卡同源（事实投影）；P4-e 起「去处理」下钻见
  // CodingAttentionOpenButton（不在卡上重复渲染表单/确认/终止）。
  coding_action: ListChecks,
} as const;

// F-50 视觉 v2 §3：门禁条目与门卡同一视觉常量（中性底+琥珀左线）；stopped 维持
// 中性弱化——琥珀不再做整卡底色。
export const KIND_CLASS = {
  gate: GATE_CARD_CLASS,
  stopped: "rounded-lg border border-slate-200 bg-gray-50 px-3 py-2",
  choice: GATE_CARD_CLASS,
  info: "rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2",
  c1_recovery: "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
  sc_failed: "rounded-lg border border-slate-200 bg-gray-50 px-3 py-2",
  lc_bootstrap: "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
  lc_resume_recovery:
    "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
  coding_action: "rounded-lg border border-amber-200 bg-amber-50 px-3 py-2",
} as const;
