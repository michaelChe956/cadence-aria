import { imageUrl } from "../api/image-create";
import type {
  ImageReferenceSource,
  ImageVendor,
  SessionRecord,
} from "../api/types/image-create";

export type ImageChatEntry =
  | UserMessage
  | ProviderText
  | PromptBlock
  | GenerationImage
  | GenerationError
  | SystemNotice
  | BusyNotice;

type ImageChatEntryBase = {
  id: string;
  content: string;
  timestamp: string;
};

export type UserMessage = ImageChatEntryBase & {
  type: "user_message";
  role: "user";
};

export type ProviderText = ImageChatEntryBase & {
  type: "provider_text";
  role: "provider";
  /**
   * 是否折叠展示。模型的 fallback 说明文本标记为 true：
   * 它只是解释为什么没产出 prompt，不该盖过提示本身。
   */
  collapsed?: boolean;
};

export type PromptBlock = ImageChatEntryBase & {
  type: "prompt_block";
  role: "provider";
  version?: number;
};

export type GenerationImage = ImageChatEntryBase & {
  type: "generation_image";
  role: "provider";
  prompt: string;
  mediaType: string;
  imageUrl: string;
  /** 供应方。旧会话记录没有该字段，读出为 undefined。 */
  vendor?: ImageVendor;
  /** 实际使用的模型 id。旧会话记录没有该字段，读出为 undefined。 */
  model?: string;
  /** 参考图来源：上一张图、上传参考图或无。 */
  referenceSource?: ImageReferenceSource;
};

export type GenerationError = ImageChatEntryBase & {
  type: "generation_error";
  role: "system";
};

export type SystemNotice = ImageChatEntryBase & {
  type: "system_notice";
  role: "system";
};

export type BusyNotice = ImageChatEntryBase & {
  type: "busy_notice";
  role: "system";
};

/**
 * 模型未产出建议 prompt 时的用户提示文案。
 *
 * - declined：模型明确未产出（拒答或主动不给），与「结构化输出没解析出来」区分；
 * - hasExplanation：本轮有没有可读的说明文本（模型是否给了 fallback 文本）；
 * - hasPrevious：是否存在保留的上一版 prompt。
 */
export function promptNoticeText(options: {
  declined: boolean;
  hasExplanation: boolean;
  hasPrevious: boolean;
}): string {
  const { declined, hasExplanation, hasPrevious } = options;
  const retained = hasPrevious ? "，已保留上一版" : "";
  if (!declined) {
    return `本轮未产出新的建议 prompt${retained}`;
  }
  const explanation = hasExplanation ? "（下方是它给出的说明）" : "";
  return `本轮模型未产出可用 prompt${explanation}${retained}`;
}

/**
 * 把提示插到 fallback 文本之前：提示比长文本更醒目，
 * 追加到末尾会被模型的长说明淹没，用户看起来像卡死。
 */
export function insertNoticeBeforeFallback(
  entries: ImageChatEntry[],
  notice: ImageChatEntry,
  fallbackEntryId: string | null,
): ImageChatEntry[] {
  const index = fallbackEntryId
    ? entries.findIndex((entry) => entry.id === fallbackEntryId)
    : -1;
  if (index < 0) {
    return [...entries, notice];
  }
  const next = [...entries];
  next.splice(index, 0, notice);
  return next;
}

/**
 * 由会话记录还原时间线：消息、提示词、生成结果与事件按时刻交织，
 * 同时刻按来源排序（消息 → 提示词 → 生成结果 → 事件）。
 */
export function buildEntries(record: SessionRecord): ImageChatEntry[] {
  const userMessages = record.messages.filter((message) => message.role === "user");
  const entries: ImageChatEntry[] = record.messages.map<ImageChatEntry>(
    (message) =>
      message.role === "user"
        ? {
            id: "",
            type: "user_message",
            role: "user",
            content: message.content,
            timestamp: message.ts,
          }
        : {
            id: "",
            type: "provider_text",
            role: "provider",
            content: message.content,
            timestamp: message.ts,
          },
  );
  entries.push(
    ...record.prompt_blocks.map<ImageChatEntry>((block, index) => ({
      id: "",
      type: "prompt_block",
      role: "provider",
      content: block.content,
      version: block.version,
      timestamp: block.ts ?? userMessages[index]?.ts ?? record.session.created_at,
    })),
    ...record.generation_results.map<ImageChatEntry>((result) => {
      if (result.image_id) {
        return {
          id: "",
          type: "generation_image",
          role: "provider",
          content: result.prompt,
          prompt: result.prompt,
          mediaType: result.media_type,
          imageUrl: imageUrl(record.session.id, result.image_id),
          // 旧会话记录没有这三个字段，读出为 undefined。
          vendor: result.vendor,
          model: result.model,
          referenceSource: result.reference_source,
          timestamp: result.ts,
        };
      }
      if (result.legacy_pending) {
        return {
          id: "",
          type: "system_notice",
          role: "system",
          content: "历史图片正在迁移，请稍后刷新",
          timestamp: result.ts,
        };
      }
      return {
        id: "",
        type: "generation_error",
        role: "system",
        content: "图片引用缺失",
        timestamp: result.ts,
      };
    }),
    ...record.events.map<ImageChatEntry>((event) => ({
      id: "",
      type:
        event.kind === "generation_error"
          ? "generation_error"
          : "system_notice",
      role: "system",
      content: event.message,
      timestamp: event.ts,
    })),
  );
  // 初始合并顺序即同时间戳优先级：消息 → 提示词 → 生成结果 → 事件。
  const ordered = entries
    .map((entry, index) => ({ entry, index, time: Date.parse(entry.timestamp) }))
    .sort((left, right) => left.time - right.time || left.index - right.index);
  if (
    record.session.current_prompt &&
    !record.prompt_blocks.some(
      (block) => block.content === record.session.current_prompt,
    )
  ) {
    // current_prompt 是当前状态而非历史事件：取最晚时刻并排在时间线末尾。
    const latest = ordered.at(-1);
    ordered.push({
      entry: {
        id: "",
        type: "prompt_block",
        role: "provider",
        content: record.session.current_prompt,
        timestamp: latest?.entry.timestamp ?? record.session.created_at,
      },
      index: ordered.length,
      time: latest?.time ?? Date.parse(record.session.created_at),
    });
  }
  return ordered.map(({ entry }, index) => {
    entry.id = `history-${record.session.id}-${index}`;
    return entry;
  });
}
