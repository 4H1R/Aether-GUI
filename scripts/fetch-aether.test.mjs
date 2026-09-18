import { test } from "node:test";
import assert from "node:assert/strict";
import { selectAsset, verifyArchive } from "./fetch-aether.mjs";

test("cross-built Intel macOS bundles contain the Intel core", () => {
  assert.equal(selectAsset("darwin", "arm64", "aether-macos-x86_64.tar.gz").name, "aether-macos-x86_64.tar.gz");
});
test("unknown hosts and unpinned assets fail closed", () => {
  assert.throws(() => selectAsset("win32", "arm64"), /Unsupported/);
  assert.throws(() => selectAsset("linux", "x64", "unverified.tar.gz"), /Unsupported/);
});
test("archive verification rejects corrupt and substituted downloads", () => {
  const hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
  verifyArchive(Buffer.from("abc"), hash);
  assert.throws(() => verifyArchive(Buffer.from("abd"), hash), /checksum mismatch/);
});
