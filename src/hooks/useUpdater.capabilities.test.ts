import { describe, expect, it } from "vitest";

// `?raw` rather than fs: the app tsconfig has no node types, and this keeps the
// capability file a build input, so moving or renaming it fails the type check
// instead of silently skipping the assertions below.
import capabilityJson from "../../src-tauri/capabilities/default.json?raw";

/**
 * Tauri's ACL is enforced at runtime, in Rust, on the real IPC boundary. Every
 * other test in this suite mocks `@tauri-apps/plugin-updater` wholesale, so the
 * ACL is never exercised and a missing permission stays invisible right up
 * until a user presses the button.
 *
 * That is not hypothetical: v1.2.6 shipped with the hook switched from
 * `downloadAndInstall()` to `download()` + `install()` while the capability
 * still only granted `allow-download-and-install`. Every platform failed with
 * "Command plugin:updater|download not allowed by ACL", and the whole test
 * suite was green.
 *
 * So this asserts the capability file and the hook agree. If you change which
 * plugin methods useUpdater.ts calls, change this list too.
 */
const REQUIRED = [
  "updater:allow-check", // runCheck -> check()
  "updater:allow-download", // install() -> update.download()
  "updater:allow-install", // relaunch() -> update.install()
  "process:allow-restart", // relaunch() -> processRelaunch()
] as const;

describe("updater capabilities", () => {
  const permissions: string[] = JSON.parse(capabilityJson).permissions;

  it.each(REQUIRED)("grants %s", (permission) => {
    expect(permissions).toContain(permission);
  });

  it("grants nothing the hook no longer uses", () => {
    // downloadAndInstall was replaced so the user sees the restart warning
    // before anything destructive happens. Leaving its permission behind would
    // let a revert pass the ACL silently, which is exactly how this broke.
    expect(permissions).not.toContain("updater:allow-download-and-install");
  });
});
