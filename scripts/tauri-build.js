#!/usr/bin/env node
// Cross-platform tauri build launcher.
// On Windows: builds C# sidecar first, then runs via msvc-wrap.bat
// On macOS/Linux: runs tauri build directly (Java sidecar pre-built by setup-db2-mac.sh)

import { execSync } from 'child_process';

if (process.platform === 'win32') {
  execSync(
    'npm run db2-sidecar:publish && scripts\\msvc-wrap.bat tauri build',
    { stdio: 'inherit', shell: true }
  );
} else {
  execSync('npx tauri build', { stdio: 'inherit', shell: true });
}
