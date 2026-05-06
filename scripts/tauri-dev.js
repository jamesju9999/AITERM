#!/usr/bin/env node
// Cross-platform tauri dev launcher.
// On Windows: builds C# sidecar first, then runs via msvc-wrap.bat
// On macOS/Linux: runs tauri dev directly (Java sidecar pre-built by setup-db2-mac.sh)

import { execSync } from 'child_process';

if (process.platform === 'win32') {
  execSync(
    'npm run db2-sidecar:publish && scripts\\msvc-wrap.bat tauri dev',
    { stdio: 'inherit', shell: true }
  );
} else {
  execSync('npx tauri dev', { stdio: 'inherit', shell: true });
}
