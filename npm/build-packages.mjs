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
  { pkg: "aiterm-host-darwin-arm64", triple: "aarch64-apple-darwin", os: "darwin", cpu: "arm64", bin: "aiterm-host" },
  { pkg: "aiterm-host-darwin-x64", triple: "x86_64-apple-darwin", os: "darwin", cpu: "x64", bin: "aiterm-host" },
  { pkg: "aiterm-host-linux-x64", triple: "x86_64-unknown-linux-musl", os: "linux", cpu: "x64", bin: "aiterm-host" },
  { pkg: "aiterm-host-linux-arm64", triple: "aarch64-unknown-linux-musl", os: "linux", cpu: "arm64", bin: "aiterm-host" },
  { pkg: "aiterm-host-win32-x64", triple: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64", bin: "aiterm-host.exe" },
];

fs.mkdirSync(outDir, { recursive: true });

for (const t of TARGETS) {
  const dir = path.join(outDir, t.pkg);
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
        name: t.pkg,
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
  console.log(`已組好 ${t.pkg}`);
}

// 入口套件：改寫版本，以及五個 optionalDependencies 的版本。
const entry = path.join(outDir, "aiterm-host");
fs.cpSync("npm/aiterm-host", entry, { recursive: true });
const pkgPath = path.join(entry, "package.json");
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
pkg.version = version;
for (const t of TARGETS) pkg.optionalDependencies[t.pkg] = version;
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
console.log(`已組好入口套件 aiterm-host@${version}`);
