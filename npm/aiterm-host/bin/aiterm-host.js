#!/usr/bin/env node
// 挑出這個平台的執行檔並交棒給它。
//
// 用 spawnSync 而不是 exec：aiterm-host 是互動式的長時間行程，要把 stdio
// 直接接通，而且結束碼要原樣傳回去。
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { packageForPlatform, binaryName } from "../lib/resolve.mjs";

const require = createRequire(import.meta.url);
const pkg = packageForPlatform(process.platform, process.arch);

let binPath;
try {
  binPath = require.resolve(`${pkg}/bin/${binaryName(process.platform)}`);
} catch {
  // optionalDependencies 在某些情況下會被整批跳過（例如 --no-optional，
  // 或安裝當下網路壞掉）。這時錯誤訊息要直接說出怎麼修，而不是丟一個
  // 裸的 MODULE_NOT_FOUND。
  console.error(
    `找不到 ${pkg}。這通常是安裝時跳過了 optional dependencies。\n` +
      `請重裝：npm install -g aiterm-host --include=optional`,
  );
  process.exit(1);
}

const result = spawnSync(binPath, process.argv.slice(2), { stdio: "inherit" });
process.exit(result.status ?? 1);
