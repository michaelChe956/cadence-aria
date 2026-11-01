// coding driver usage 采集测试（usage 接通/mjs 面）。
//
// 纯 Node：mock WS 事件序列喂 collectUsageByRole + codingUsageResult，
// 断言 result JSON 的 usage_by_role / usage 输出。事件形态对齐服务端
// coding provider stream 实际发射的 CodingExecutionEvent
// （event.kind=usage、event_id=usage_{role}、output=UsageReportData JSON、
// 同 role 多报 upsert 覆盖最新快照）。
import assert from 'node:assert/strict';
import test from 'node:test';

import { codingUsageResult } from './coding_run_campaign.mjs';
import { collectUsageByRole } from './workitem_run_campaign.mjs';

function codingUsageEvent(role, tokens) {
  return {
    type: 'coding_execution_event',
    event: {
      event_id: `usage_${role}`,
      node_id: 'timeline_node_002',
      agent: 'pi',
      kind: 'usage',
      status: 'completed',
      title: `${role} token usage`,
      detail: null,
      command: null,
      cwd: null,
      output: JSON.stringify({ role, ...tokens }),
      exit_code: null,
    },
  };
}

function collectAll(messages) {
  const usageByRole = {};
  messages.forEach((message) => collectUsageByRole(message, usageByRole));
  return usageByRole;
}

test('单次 usage 事件：usage_by_role 按角色原样值，usage 汇总兼容既有扁平 schema', () => {
  const usageByRole = collectAll([
    codingUsageEvent('author', { input_tokens: 1200, output_tokens: 80, cache_read_tokens: 600, cache_creation_tokens: null }),
  ]);
  const result = codingUsageResult(usageByRole);
  assert.deepEqual(result.usage_by_role, {
    author: { input_tokens: 1200, output_tokens: 80, cache_read_tokens: 600, cache_creation_tokens: null },
  });
  assert.deepEqual(result.usage, { input_tokens: 1200, output_tokens: 80, cache_read_tokens: 600 });
});

test('同 role 多次上报（upsert 语义）：last-wins 且扁平汇总不得重复计入旧快照', () => {
  const usageByRole = collectAll([
    codingUsageEvent('author', { input_tokens: 1200, output_tokens: 80, cache_read_tokens: 600, cache_creation_tokens: null }),
    codingUsageEvent('author', { input_tokens: 1500, output_tokens: 120, cache_read_tokens: 0, cache_creation_tokens: 30 }),
  ]);
  const result = codingUsageResult(usageByRole);
  // last-wins：仅保留最新快照，而非两快照求和（1500 而非 2700）。
  assert.deepEqual(result.usage_by_role.author, { input_tokens: 1500, output_tokens: 120, cache_read_tokens: 0, cache_creation_tokens: 30 });
  assert.deepEqual(result.usage, { input_tokens: 1500, output_tokens: 120, cache_read_tokens: 0 });
});

test('author 与 reviewer 双角色：分键保留，扁平汇总为两角色最新快照之和', () => {
  const usageByRole = collectAll([
    codingUsageEvent('author', { input_tokens: 1000, output_tokens: 100, cache_read_tokens: 50, cache_creation_tokens: 10 }),
    codingUsageEvent('reviewer', { input_tokens: 24000, output_tokens: 500, cache_read_tokens: 700, cache_creation_tokens: 0 }),
  ]);
  const result = codingUsageResult(usageByRole);
  assert.deepEqual(Object.keys(result.usage_by_role).sort(), ['author', 'reviewer']);
  assert.deepEqual(result.usage, { input_tokens: 25000, output_tokens: 600, cache_read_tokens: 750 });
});

test('畸形 output：fail-closed 于采集，不从其他字段推断 token 用量', () => {
  const malformed = {
    type: 'coding_execution_event',
    event: {
      event_id: 'usage_author',
      kind: 'usage',
      status: 'completed',
      output: 'not-json',
      input_tokens: 999,
    },
  };
  const usageByRole = collectAll([malformed]);
  assert.deepEqual(usageByRole, {});
});

test('零事件：usage 与 usage_by_role 均回落 usage_unavailable: true', () => {
  const result = codingUsageResult(collectAll([
    { type: 'coding_session_state', stage: 'coding' },
    { type: 'coding_stream_chunk', content: 'partial' },
  ]));
  assert.deepEqual(result.usage, { usage_unavailable: true });
  assert.deepEqual(result.usage_by_role, { usage_unavailable: true });
});

test('null token 字段：usage_by_role 原样保留 null，扁平汇总按 0 计', () => {
  const usageByRole = collectAll([
    codingUsageEvent('author', { input_tokens: 1200, output_tokens: null, cache_read_tokens: null, cache_creation_tokens: null }),
  ]);
  const result = codingUsageResult(usageByRole);
  assert.equal(result.usage_by_role.author.output_tokens, null);
  assert.equal(result.usage_by_role.author.cache_read_tokens, null);
  assert.deepEqual(result.usage, { input_tokens: 1200, output_tokens: 0, cache_read_tokens: 0 });
});
