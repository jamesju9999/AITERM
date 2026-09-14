#!/usr/bin/env node
// 從已下載的 release artifact 組出所有 npm 套件目錄。
//
// 用法：node npm/build-packages.mjs <version> <artifact-dir> <out-dir>
//
// <artifact-dir> 裡要有解開後的 aiterm-host-<version>-<triple>/ 目錄。
import fs from "node:fs";
import path from "node:path";

const [version, artifactDir, outDir] = process.argv.slice(2);
if (!version || !artifactDir || !outDir) {
  console.error("用法：node npm/build-packages.mjs <version> <artifact-dir> <out-dir>");
  process.exit(2);
}

const TARGETS = [
  { dir: "aiterm-host-darwin-arm64", triple: "aarch64-apple-darwin", os: "darwin", cpu: "arm64", bin: "aiterm-host" },
  { dir: "aiterm-host-darwin-x64", triple: "x86_64-apple-darwin", os: "darwin", cpu: "x64", bin: "aiterm-host" },
  { dir: "aiterm-host-linux-x64", triple: "x86_64-unknown-linux-musl", os: "linux", cpu: "x64", bin: "aiterm-host" },
  { dir: "aiterm-host-linux-arm64", triple: "aarch64-unknown-linux-musl", os: "linux", cpu: "arm64", bin: "aiterm-host" },
  { dir: "aiterm-host-win32-x64", triple: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64", bin: "aiterm-host.exe" },
];

fs.mkdirSync(outDir, { recursive: true });

// 平台套件一律帶範圍。不帶範圍的 `aiterm-host-win32-x64` 被 npm 的名稱防濫用
// 機制擋下（`403 Package name triggered spam detection`），隔天、不同的 run
// 重試仍然一樣——不是速率限制，是名稱本身被判定有問題。帶範圍的名稱在自己的
// 命名空間裡不會觸發。
//
// **目錄名刻意維持不帶範圍**：CI 的發佈迴圈用 `npm-dist/aiterm-host-*` 這個
// glob 找目錄，改成巢狀的 `@scope/...` 會讓它一個都找不到。
const SCOPE = "@jamesju";
const npmName = (t) => `${SCOPE}/${t.dir}`;

for (const t of TARGETS) {
  const dir = path.join(outDir, t.dir);
  fs.mkdirSync(path.join(dir, "bin"), { recursive: true });

  const src = path.join(artifactDir, `aiterm-host-${version}-${t.triple}`, t.bin);
  if (!fs.existsSync(src)) {
    // 少一個平台就整個失敗。悄悄跳過的話，入口套件的 optionalDependencies
    // 會指向一個不存在的版本，那個平台的使用者安裝時才會爆。
    console.error(`找不到執行檔：${src}`);
    process.exit(1);
  }
  fs.copyFileSync(src, path.join(dir, "bin", t.bin));
  fs.chmodSync(path.join(dir, "bin", t.bin), 0o755);

  fs.writeFileSync(
    path.join(dir, "package.json"),
    JSON.stringify(
      {
        name: npmName(t),
        version,
        description: `aiterm-host binary for ${t.os} ${t.cpu}`,
        os: [t.os],
        cpu: [t.cpu],
        files: ["bin"],
        license: "Apache-2.0",
        repository: { type: "git", url: "git+https://github.com/jamesju9999/AITERM.git" },
      },
      null,
      2,
    ) + "\n",
  );
  console.log(`已組好 ${npmName(t)}`);
}

// 入口套件：改寫版本，以及五個 optionalDependencies 的版本。
const entry = path.join(outDir, "aiterm-host");
fs.cpSync("npm/aiterm-host", entry, { recursive: true });
const pkgPath = path.join(entry, "package.json");
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
pkg.version = version;
pkg.optionalDependencies = {};
for (const t of TARGETS) pkg.optionalDependencies[npmName(t)] = version;
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
console.log(`已組好入口套件 aiterm-host@${version}`);
