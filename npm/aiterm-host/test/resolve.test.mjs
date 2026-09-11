// 平台解析的測試。用 node 內建的 test runner，不拉任何相依——這個套件
// 本身要盡量輕，它只是一個下載器的殼。
//
// 執行：node --test npm/aiterm-host/test/
import { test } from "node:test";
import assert from "node:assert/strict";
import { packageForPlatform, binaryName } from "../lib/resolve.mjs";

test("Apple Silicon", () => {
  assert.equal(packageForPlatform("darwin", "arm64"), "aiterm-host-darwin-arm64");
});

test("Intel Mac", () => {
  assert.equal(packageForPlatform("darwin", "x64"), "aiterm-host-darwin-x64");
});

test("Linux x64", () => {
  assert.equal(packageForPlatform("linux", "x64"), "aiterm-host-linux-x64");
});

test("Linux arm64", () => {
  assert.equal(packageForPlatform("linux", "arm64"), "aiterm-host-linux-arm64");
});

test("Windows x64", () => {
  assert.equal(packageForPlatform("win32", "x64"), "aiterm-host-win32-x64");
});

test("不支援的平台要丟出可讀的錯誤，不是回 undefined", () => {
  // 回 undefined 的話，後面的 require 會噴一個跟根因無關的
  // "Cannot find module undefined"，使用者完全不知道發生什麼事。
  assert.throws(
    () => packageForPlatform("freebsd", "x64"),
    /freebsd.*x64/,
    "錯誤訊息要包含實際的平台與架構",
  );
});

test("Windows 上的執行檔名要帶 .exe", () => {
  assert.equal(binaryName("win32"), "aiterm-host.exe");
  assert.equal(binaryName("linux"), "aiterm-host");
  assert.equal(binaryName("darwin"), "aiterm-host");
});
