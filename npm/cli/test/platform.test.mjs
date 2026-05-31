import { test } from "node:test";
import assert from "node:assert/strict";
import platform from "../lib/platform.js";

const { subpackageName, SUPPORTED } = platform;

test("maps linux x64 to subpackage", () => {
  assert.equal(subpackageName("linux", "x64"), "@cadence-aria/cli-linux-x64");
});

test("maps darwin x64 to subpackage", () => {
  assert.equal(subpackageName("darwin", "x64"), "@cadence-aria/cli-darwin-x64");
});

test("maps darwin arm64 to subpackage", () => {
  assert.equal(subpackageName("darwin", "arm64"), "@cadence-aria/cli-darwin-arm64");
});

test("unsupported platform throws with clear message", () => {
  assert.throws(() => subpackageName("win32", "x64"), /unsupported|不支持|win32-x64/i);
});

test("SUPPORTED lists the three target triples", () => {
  assert.deepEqual([...SUPPORTED].sort(), ["darwin-arm64", "darwin-x64", "linux-x64"]);
});
