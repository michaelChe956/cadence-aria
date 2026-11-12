import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getImageCreateSession } from "../api/image-create";
import type * as ImageCreateApi from "../api/image-create";
import {
  DEFAULT_IMAGE_PARAMS,
  type ImageGenerationResult,
  type SessionRecord,
} from "../api/types/image-create";
import { useImageCreateStore } from "./image-create-store";

vi.mock("../api/image-create", async (importOriginal) => ({
  ...(await importOriginal<typeof ImageCreateApi>()),
  getImageCreateSession: vi.fn(),
}));

const CREATED_AT = "2026-11-12T05:20:00Z";

function generationResult(
  overrides: Partial<ImageGenerationResult> & { ts: string },
): ImageGenerationResult {
  return {
    prompt: "一只小狗",
    params: DEFAULT_IMAGE_PARAMS,
    media_type: "image/png",
    image_id: "image-1",
    legacy_pending: false,
    vendor: "minimax",
    model: "MiniMax-H3-Image",
    reference_source: "none",
    ...overrides,
  };
}

function record(overrides: Partial<SessionRecord> = {}): SessionRecord {
  return {
    session: {
      id: "session-1",
      provider_name: "claude_code",
      template: { preset: "ppt_business_illustration" },
      last_provider_session_id: null,
      current_prompt: null,
      status: "active",
      created_at: CREATED_AT,
    },
    messages: [],
    prompt_blocks: [],
    generation_results: [],
    events: [],
    generation: 0,
    ...overrides,
  };
}

async function openRecord(value: SessionRecord) {
  vi.mocked(getImageCreateSession).mockResolvedValue(value);
  await useImageCreateStore.getState().openSession(value.session.id);
  return useImageCreateStore.getState().entries;
}

/** 按用户提供的三轮会话时间线构造，提示词时刻为测试数据。 */
function interleavedRecord(): SessionRecord {
  return record({
    messages: [
      { role: "user", content: "我期望你帮我生成一个小狗", ts: "2026-11-12T05:23:53Z" },
      { role: "user", content: "帮我把这个金毛改的像特朗普一点", ts: "2026-11-12T06:04:39Z" },
      { role: "user", content: "脸还不够川普，再画的像一点", ts: "2026-11-12T06:07:26Z" },
    ],
    prompt_blocks: [
      { content: "一只可爱的小狗", version: 1, ts: "2026-11-12T05:23:55Z" },
      { content: "金毛，特朗普风格", version: 2, ts: "2026-11-12T06:04:41Z" },
      { content: "更像特朗普一点", version: 3, ts: "2026-11-12T06:07:28Z" },
    ],
    generation_results: [
      generationResult({ prompt: "一只可爱的小狗", image_id: "image-1", ts: "2026-11-12T05:25:03Z" }),
      generationResult({ prompt: "金毛，特朗普风格", image_id: "image-2", vendor: "gpt", model: "gpt-image-2.6-flare", reference_source: "previous", ts: "2026-11-12T06:06:20Z" }),
      generationResult({ prompt: "更像特朗普一点", image_id: "image-3", reference_source: "previous", ts: "2026-11-12T06:18:37Z" }),
    ],
    events: [
      { kind: "generation_error", message: "MiniMax 限流（429）", ts: "2026-11-12T06:09:17Z" },
    ],
  });
}

describe("会话历史按时间交织", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useImageCreateStore.getState().reset();
    // 仅隔离网络连接，历史重建走实际 openSession 与 buildEntries。
    vi.stubGlobal("WebSocket", class {
      close() {}
    });
  });

  afterEach(() => {
    useImageCreateStore.getState().reset();
    vi.unstubAllGlobals();
  });

  it("把消息、提示词、生成结果与事件按时间串成一条时间线", async () => {
    const entries = await openRecord(interleavedRecord());
    expect(entries.map((entry) => [entry.type, entry.content])).toEqual([
      ["user_message", "我期望你帮我生成一个小狗"],
      ["prompt_block", "一只可爱的小狗"],
      ["generation_image", "一只可爱的小狗"],
      ["user_message", "帮我把这个金毛改的像特朗普一点"],
      ["prompt_block", "金毛，特朗普风格"],
      ["generation_image", "金毛，特朗普风格"],
      ["user_message", "脸还不够川普，再画的像一点"],
      ["prompt_block", "更像特朗普一点"],
      ["generation_error", "MiniMax 限流（429）"],
      ["generation_image", "更像特朗普一点"],
    ]);
  });

  it("提示词块使用自身时间戳而非会话创建时间", async () => {
    const entries = await openRecord(interleavedRecord());
    expect(entries.filter((entry) => entry.type === "prompt_block").map((entry) => entry.timestamp)).toEqual([
      "2026-11-12T05:23:55Z",
      "2026-11-12T06:04:41Z",
      "2026-11-12T06:07:28Z",
    ]);
  });

  it("同一时刻按来源排序且保留每类内部次序，包括缺图与迁移结果", async () => {
    const ts = "2026-11-12T07:00:00Z";
    const entries = await openRecord(record({
      messages: [
        { role: "user", content: "用户消息", ts },
        { role: "provider", content: "回复", ts },
      ],
      prompt_blocks: [
        { content: "提示一", version: 1, ts },
        { content: "提示二", version: 2, ts },
      ],
      generation_results: [
        generationResult({ prompt: "结果一", ts }),
        generationResult({ image_id: null, legacy_pending: true, ts }),
        generationResult({ image_id: null, ts }),
      ],
      events: [
        { kind: "generation_error", message: "报错事件", ts },
        { kind: "notice", message: "提示事件", ts },
      ],
    }));
    expect(entries.map((entry) => entry.content)).toEqual([
      "用户消息", "回复", "提示一", "提示二", "结果一",
      "历史图片正在迁移，请稍后刷新", "图片引用缺失", "报错事件", "提示事件",
    ]);
  });

  it("缺省和 null 时间戳按同序号用户消息兜底，忽略供应方回复", async () => {
    const entries = await openRecord(record({
      messages: [
        { role: "user", content: "第一条需求", ts: "2026-11-12T05:23:53Z" },
        { role: "provider", content: "第一轮回复", ts: "2026-11-12T05:24:00Z" },
        { role: "user", content: "第二条需求", ts: "2026-11-12T06:04:39Z" },
      ],
      prompt_blocks: [
        { content: "旧提示词一", version: 1 },
        { content: "旧提示词二", version: 2, ts: null },
      ],
    }));
    expect(entries.map((entry) => [entry.content, entry.timestamp])).toEqual([
      ["第一条需求", "2026-11-12T05:23:53Z"],
      ["旧提示词一", "2026-11-12T05:23:53Z"],
      ["第一轮回复", "2026-11-12T05:24:00Z"],
      ["第二条需求", "2026-11-12T06:04:39Z"],
      ["旧提示词二", "2026-11-12T06:04:39Z"],
    ]);
  });

  it("新旧提示词混合时仍按块的原始序号匹配用户消息", async () => {
    const value = interleavedRecord();
    delete value.prompt_blocks[1].ts;
    const entries = await openRecord(value);
    expect(entries.find((entry) => entry.type === "prompt_block" && entry.version === 2)?.timestamp)
      .toBe("2026-11-12T06:04:39Z");
  });

  it("缺少同序号用户消息的旧块回退到创建时间并参与排序", async () => {
    const entries = await openRecord(record({
      messages: [{ role: "user", content: "唯一需求", ts: "2026-11-12T06:00:00Z" }],
      prompt_blocks: [
        { content: "有对应消息", version: 1 },
        { content: "无对应消息", version: 2 },
      ],
    }));
    expect(entries.map((entry) => [entry.content, entry.timestamp])).toEqual([
      ["无对应消息", CREATED_AT],
      ["唯一需求", "2026-11-12T06:00:00Z"],
      ["有对应消息", "2026-11-12T06:00:00Z"],
    ]);
  });

  it("以实际时刻排序而非 ISO 字面序，且不改变原始记录", async () => {
    const value = record({
      messages: [
        { role: "user", content: "晚消息", ts: "2026-11-12T05:00:00Z" },
        { role: "user", content: "早消息", ts: "2026-11-12T12:00:00+08:00" },
      ],
      events: [{ kind: "notice", message: "中间事件", ts: "2026-11-12T04:30:00Z" }],
    });
    const original = structuredClone(value);
    const entries = await openRecord(value);
    expect(entries.map((entry) => entry.content)).toEqual(["早消息", "中间事件", "晚消息"]);
    expect(value).toEqual(original);
  });

  it("排序后的 id 唯一，重开同一会话保持稳定，跨会话不会碰撞", async () => {
    const value = interleavedRecord();
    const ids = (await openRecord(value)).map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect((await openRecord(value)).map((entry) => entry.id)).toEqual(ids);
    const other = await openRecord({ ...value, session: { ...value.session, id: "另一会话" } });
    expect(other.every((entry) => !ids.includes(entry.id))).toBe(true);
  });

  it("仅 current_prompt 保存的旧提示词也按创建时间加入提示词组", async () => {
    const value = record({
      messages: [{ role: "user", content: "晚消息", ts: "2026-11-12T07:00:00Z" }],
      generation_results: [generationResult({ prompt: "创建时的图", ts: CREATED_AT })],
      events: [{ kind: "notice", message: "创建时的事件", ts: CREATED_AT }],
    });
    value.session.current_prompt = "仅当前提示词";
    const entries = await openRecord(value);
    expect(entries.map((entry) => entry.content)).toEqual([
      "仅当前提示词", "创建时的图", "创建时的事件", "晚消息",
    ]);
  });

  it("已有同内容的提示词块时不重复补 current_prompt", async () => {
    const value = interleavedRecord();
    value.session.current_prompt = value.prompt_blocks[2].content;
    const entries = await openRecord(value);
    expect(entries.filter((entry) => entry.type === "prompt_block")).toHaveLength(3);
  });
});
