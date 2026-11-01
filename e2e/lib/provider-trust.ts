import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

/// Codex 用户级 projects trust(矩阵 ensure_provider_trust 同款写面):
/// `~/.codex/config.toml` 中 `[projects."<canonical_root>"]` 的
/// `trust_level = "trusted"`。命令行 `-c` 覆盖不参与 codex trust 判定,
/// 登记必须落用户级配置文件(预研报告 2026-10-01 实证)。
///
/// 语义镜像产品 CodexTrustAdapter(provider_trust_adapters.rs):
/// - 表缺失 → 追加表头+trust_level;表在而键缺 → 表头后插入;
/// - 已 trusted → 幂等 no-op;trust_level 为其他值 → 拒绝覆盖(冲突);
/// - 原子写(同目录 tmp+rename);记录 before/after 摘要。
/// /tmp 强制规则豁免口径:只动 HOME 的既有 trust 面(与矩阵同款),
/// 不在 HOME 其他位置创建测试内容。

export type CodexTrustOutcome = {
  canonicalRoot: string;
  action: "created" | "already_trusted";
  beforeDigest: string | null;
  afterDigest: string;
};

function digestText(content: string): string {
  return createHash("sha256").update(content, "utf8").digest("hex");
}

/** TOML basic string 转义(镜像 escape_toml_basic_string:反斜杠/引号/控制字符)。 */
function escapeTomlBasicString(value: string): string {
  return value
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, (char) =>
      `\\u${char.charCodeAt(0).toString(16).padStart(4, "0")}`,
    );
}

function tableHeader(canonicalRoot: string): string {
  return `[projects."${escapeTomlBasicString(canonicalRoot)}"]`;
}

function isTableHeaderLine(line: string): boolean {
  const trimmed = line.trimStart();
  return trimmed.startsWith("[") && !trimmed.startsWith("[[");
}

/**
 * 定位 `[projects."<root>"]` 表;返回 (表头行号, 表体结束行号(不含))。
 * 表体延伸到下一个表头行或文件末尾(与 locate_projects_table 同口径)。
 */
function locateProjectsTable(lines: string[], header: string): [number, number] | null {
  for (let index = 0; index < lines.length; index += 1) {
    if (lines[index]!.trim() === header) {
      let end = index + 1;
      while (end < lines.length && !isTableHeaderLine(lines[end]!)) end += 1;
      return [index, end];
    }
  }
  return null;
}

function trustLevelInTable(lines: string[], table: [number, number]): string | null {
  for (let index = table[0] + 1; index < table[1]; index += 1) {
    const match = /^trust_level\s*=\s*"([^"]*)"\s*$/.exec(lines[index]!.trim());
    if (match) return match[1] ?? "";
  }
  return null;
}

function atomicWriteText(target: string, content: string): void {
  mkdirSync(path.dirname(target), { recursive: true });
  const temp = `${target}.aria-e2e-${process.pid}-${Date.now()}.tmp`;
  writeFileSync(temp, content, "utf8");
  renameSync(temp, target);
}

export function codexTrustConfigPath(): string {
  return path.join(os.homedir(), ".codex", "config.toml");
}

/**
 * 为 canonical root 登记 codex trust(幂等;冲突拒绝覆盖用户既有值)。
 * beforeDigest 供 CAS 语义审计;调用方(装配)串行执行,单写者。
 */
export function ensureCodexTrust(canonicalRoot: string): CodexTrustOutcome {
  const configPath = codexTrustConfigPath();
  const header = tableHeader(canonicalRoot);
  const content = existsSync(configPath) ? readFileSync(configPath, "utf8") : null;
  const beforeDigest = content === null ? null : digestText(content);

  if (content === null) {
    const fresh = `${header}\ntrust_level = "trusted"\n`;
    atomicWriteText(configPath, fresh);
    return { canonicalRoot, action: "created", beforeDigest: null, afterDigest: digestText(fresh) };
  }

  const lines = content.split("\n");
  const table = locateProjectsTable(lines, header);
  if (table === null) {
    let updated = content;
    if (updated.length > 0 && !updated.endsWith("\n")) updated += "\n";
    updated += `${header}\ntrust_level = "trusted"\n`;
    atomicWriteText(configPath, updated);
    return { canonicalRoot, action: "created", beforeDigest, afterDigest: digestText(updated) };
  }

  const existing = trustLevelInTable(lines, table);
  if (existing === "trusted") {
    return { canonicalRoot, action: "already_trusted", beforeDigest, afterDigest: beforeDigest! };
  }
  if (existing !== null) {
    throw new Error(
      `codex trust 冲突:${header} 已有 trust_level = ${JSON.stringify(existing)},拒绝覆盖用户既有值(产品同款 CAS 拒绝语义);请人工确认该键后重试`,
    );
  }
  const updated = [...lines.slice(0, table[0] + 1), 'trust_level = "trusted"', ...lines.slice(table[0] + 1)].join("\n");
  atomicWriteText(configPath, updated);
  return { canonicalRoot, action: "created", beforeDigest, afterDigest: digestText(updated) };
}

/** 只读核验(不动文件):返回当前 trust 状态。 */
export function readCodexTrustState(canonicalRoot: string): "trusted" | "untrusted" | "missing" | "no_config" {
  const configPath = codexTrustConfigPath();
  if (!existsSync(configPath)) return "no_config";
  const lines = readFileSync(configPath, "utf8").split("\n");
  const table = locateProjectsTable(lines, tableHeader(canonicalRoot));
  if (table === null) return "missing";
  const level = trustLevelInTable(lines, table);
  return level === "trusted" ? "trusted" : "untrusted";
}
