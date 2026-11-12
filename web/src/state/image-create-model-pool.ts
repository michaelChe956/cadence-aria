import type { MaskedVendorSettings } from "../api/types/image-create";

/** 单家供应方的模型池：网关返回的模型与可勾选的候选池。 */
export type ImageVendorModelPool = {
  /** 网关模型列表：最近一次探测 /v1/models 成功时返回的模型 id。 */
  gateway: string[];
  /** 候选池：网关模型 ∪ 已配置项（含自定义添加），按池顺序。 */
  candidates: string[];
};

/** 单家供应方的模型清单上限：网关模型过多时只保留前 N 个。 */
export const IMAGE_MODELS_LIMIT = 256;

/** 候选池刷新结果：新候选池、新勾选集合与需要提示用户的注意事项。 */
export type ModelCandidatesRefresh = {
  /** 新的候选池：网关模型（网关顺序在前）∪ 草稿清单里已有的。 */
  candidates: string[];
  /** 新的勾选集合：网关返回的模型（去重、保序、跳过另一家占用的 id）。 */
  selected: string[];
  /** 已被另一家勾选而跳过的模型 id。 */
  skipped: string[];
  /** 自动勾选时因超过单家上限而截断：网关模型总数与实际保留数。 */
  truncated: { total: number; kept: number } | null;
};

/** 模型 id 列表去重：去掉首尾空白与空串，保留首次出现的顺序。 */
export function normalizeModels(models: readonly string[]): string[] {
  const result: string[] = [];
  for (const model of models) {
    const id = model.trim();
    if (id !== "" && !result.includes(id)) {
      result.push(id);
    }
  }
  return result;
}

/** 按候选池顺序排列勾选集合：池内的按池序，池外的追加在后。 */
export function orderByPool(
  pool: readonly string[],
  selected: readonly string[],
): string[] {
  const chosen = new Set(selected);
  const ordered = pool.filter((model) => chosen.has(model));
  for (const model of selected) {
    if (!pool.includes(model)) {
      ordered.push(model);
    }
  }
  return ordered;
}

/**
 * 用网关模型刷新候选池与草稿清单（纯计算）：
 * - 候选池 = 网关模型（网关顺序在前）∪ 草稿清单里已有的（网关没返回的追加在后）；
 * - 草稿清单 = 网关返回的模型（去重、保序），已被另一家勾选占用的 id 跳过，
 *   超过单家上限时只保留前 N 个；
 * - 已知取舍：用户取消勾选过的模型会再次被勾上，自定义但网关没返回的 id 会移出清单。
 */
export function refreshCandidatePool(
  gateway: readonly string[],
  current: readonly string[],
  otherSelected: readonly string[],
): ModelCandidatesRefresh {
  const gatewayModels = normalizeModels(gateway);
  const currentModels = normalizeModels(current);
  const used = new Set(normalizeModels(otherSelected));
  // 已被另一家勾选的模型 id 归属那一家，本家跳过。
  const skipped = gatewayModels.filter((model) => used.has(model));
  const available = gatewayModels.filter((model) => !used.has(model));
  const candidates = normalizeModels([
    ...available,
    ...currentModels.filter((model) => !used.has(model)),
  ]);
  const kept = available.slice(0, IMAGE_MODELS_LIMIT);
  const truncated =
    available.length > IMAGE_MODELS_LIMIT
      ? { total: available.length, kept: kept.length }
      : null;
  return { candidates, selected: kept, skipped, truncated };
}

/**
 * 把额外的模型 id 并入候选池并勾选（纯计算）：
 * 候选池 = 池里已有的 ∪ 草稿清单 ∪ 并入的 id；
 * 勾选集合 = 草稿清单 ∪ 并入的 id，按候选池顺序排列。
 */
export function mergeCandidatePool(
  pool: readonly string[],
  current: readonly string[],
  incoming: readonly string[],
): { candidates: string[]; selected: string[] } {
  const selected = normalizeModels(current);
  const additions = normalizeModels(incoming);
  const candidates = normalizeModels([...pool, ...selected, ...additions]);
  return {
    candidates,
    selected: orderByPool(
      candidates,
      normalizeModels([...selected, ...additions]),
    ),
  };
}

/**
 * 单家的初始选中项（三级优先）：
 * 1. 内置推荐清单的首个 id 若在当前清单里 → 选它；
 * 2. 否则清单里第一个含 "image"（忽略大小写）的 id；
 * 3. 否则取清单首项。
 */
export function initialVendorModel(
  vendorSettings: MaskedVendorSettings,
): string {
  const models = vendorSettings.models;
  if (models.length === 0) {
    return "";
  }
  const recommended = vendorSettings.recommended_models[0];
  if (recommended && models.includes(recommended)) {
    return recommended;
  }
  const imageModel = models.find((model) =>
    model.toLowerCase().includes("image"),
  );
  return imageModel ?? models[0];
}
