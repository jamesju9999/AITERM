#!/usr/bin/env node
// Cross-platform tauri build launcher.
// On Windows: builds C# sidecar first, then runs via msvc-wrap.bat
// On macOS: runs tauri build, then copies db2-sidecar into the .app bundle
//           (Tauri's glob resources flatten directory trees, so we copy manually)
// On Linux: runs tauri build directly

import { execSync } from 'child_process';
import { cpSync, existsSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = join(__dirname, '..');

if (process.platform === 'win32') {
  execSync(
    'npm run db2-sidecar:publish && scripts\\msvc-wrap.bat tauri build',
    { stdio: 'inherit', shell: true }
  );
} else if (process.platform === 'darwin') {
  execSync('npx tauri build', { stdio: 'inherit', shell: true });

  // Copy db2-sidecar into the built .app bundle preserving directory structure.
  // Tauri's glob resource bundling flattens nested directories, so we do this manually.
  const arch = process.arch === 'arm64' ? 'db2-sidecar-mac-arm64' : 'db2-sidecar-mac-x64';
  const sidecarSrc = join(projectRoot, 'src-tauri', 'binaries', arch);
  const appBundle = join(projectRoot, 'src-tauri', 'target', 'release', 'bundle', 'macos', 'AITerm.app');
  const sidecarDest = join(appBundle, 'Contents', 'Resources', 'db2-sidecar');

  if (existsSync(sidecarSrc) && existsSync(appBundle)) {
    console.log(`Copying ${arch} → Contents/Resources/db2-sidecar/`);
    cpSync(sidecarSrc, sidecarDest, { recursive: true, force: true });
    console.log('db2-sidecar bundled successfully.');
  } else if (!existsSync(sidecarSrc)) {
    console.warn(`Warning: ${sidecarSrc} not found — DB2 will not work in this build.`);
  }
} else {
  execSync('npx tauri build', { stdio: 'inherit', shell: true });
}
