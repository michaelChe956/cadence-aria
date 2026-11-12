import type {
  ImageReferenceSource,
  ImageVendor,
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
