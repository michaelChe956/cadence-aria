import { test } from "node:test";
import assert from "node:assert/strict";
import args from "../lib/args.js";

const { planInvocation } = args;

test("no args injects default web subcommand and enables open", () => {
  const plan = planInvocation([]);
  assert.deepEqual(plan.forwardArgs, ["web"]);
  assert.equal(plan.defaultWebMode, true);
  assert.equal(plan.open, true);
});

test("explicit web subcommand without port keeps default web mode", () => {
  const plan = planInvocation(["web"]);
  assert.deepEqual(plan.forwardArgs, ["web"]);
  assert.equal(plan.defaultWebMode, true);
});

test("explicit web --port disables auto port and open", () => {
  const plan = planInvocation(["web", "--port", "3000"]);
  assert.deepEqual(plan.forwardArgs, ["web", "--port", "3000"]);
  assert.equal(plan.defaultWebMode, false);
  assert.equal(plan.open, false);
});

test("--no-open is stripped from forwarded args and disables open", () => {
  const plan = planInvocation(["--no-open"]);
  assert.deepEqual(plan.forwardArgs, ["web"]); // 无参 -> 注入 web；--no-open 被剥离
  assert.equal(plan.open, false);
  assert.equal(plan.defaultWebMode, true);
});

test("non-web subcommand forwarded verbatim, no open", () => {
  const plan = planInvocation(["task", "run", "--workspace", "/tmp/x"]);
  assert.deepEqual(plan.forwardArgs, ["task", "run", "--workspace", "/tmp/x"]);
  assert.equal(plan.defaultWebMode, false);
  assert.equal(plan.open, false);
});

test("web with any flag (e.g. --check) is NOT default web mode, forwarded verbatim", () => {
  // 回归：web --check 曾被误判为默认 web 模式,导致 launcher 劫持端口起真实服务、忽略 --check。
  const plan = planInvocation(["web", "--check", "--workspace", "/tmp/x"]);
  assert.deepEqual(plan.forwardArgs, ["web", "--check", "--workspace", "/tmp/x"]);
  assert.equal(plan.defaultWebMode, false);
  assert.equal(plan.open, false);
});

test("web with --host only is also non-default (user takes over)", () => {
  const plan = planInvocation(["web", "--host", "0.0.0.0"]);
  assert.deepEqual(plan.forwardArgs, ["web", "--host", "0.0.0.0"]);
  assert.equal(plan.defaultWebMode, false);
  assert.equal(plan.open, false);
});
