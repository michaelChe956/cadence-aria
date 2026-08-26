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
