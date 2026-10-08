import { describe, it, expect } from "vitest";
import { parseRemoteLogin, isLogoutCommand } from "./remoteSession";

describe("parseRemoteLogin", () => {
  it.each([
    ["ssh jamesju@192.168.1.80", "jamesju@192.168.1.80"],
    ["ssh -p 2222 admin@nas.local", "admin@nas.local"],
    ["ssh -o StrictHostKeyChecking=no -i ~/.ssh/k jamesju@10.0.0.5", "jamesju@10.0.0.5"],
    ["sshpass -p 'x y' ssh -o StrictHostKeyChecking=no jamesju@192.168.1.80", "jamesju@192.168.1.80"],
    ["SSHPASS='pw' sshpass -e ssh -o StrictHostKeyChecking=accept-new jamesju@192.168.1.80", "jamesju@192.168.1.80"],
    ["telnet 192.168.1.1", "192.168.1.1"],
    ["telnet router.lan 2323", "router.lan"],
    ["mosh user@host", "user@host"],
    ["ssh myserver", "myserver"],
  ])("%s -> %s", (cmd, target) => {
    expect(parseRemoteLogin(cmd)?.target).toBe(target);
  });

  it.each([
    "ssh jamesju@host 'hostname -I'",
    "ssh host ls -la",
    "SSHPASS=x sshpass -e ssh -o A=b u@h \"hostname; ip a\"",
    "ssh -N -L 8080:localhost:80 user@host",
    "ssh-keygen -t ed25519",
    "scp a b@c:/tmp",
    "echo ssh user@host",
    "ls",
    "",
  ])("one-shot / unrelated is null: %s", (cmd) => {
    expect(parseRemoteLogin(cmd)).toBeNull();
  });
});

describe("isLogoutCommand", () => {
  it("accepts exit / logout (with optional code)", () => {
    expect(isLogoutCommand("exit")).toBe(true);
    expect(isLogoutCommand(" logout ")).toBe(true);
    expect(isLogoutCommand("exit 2")).toBe(true);
  });
  it("rejects things that merely start with exit", () => {
    expect(isLogoutCommand("exitfoo")).toBe(false);
    expect(isLogoutCommand("echo exit")).toBe(false);
  });
});

import { remoteSessionPromptNote } from "./remoteSession";
describe("remoteSessionPromptNote", () => {
  it("沒有遠端工作階段時是空字串（不影響原本提示詞）", () => {
    expect(remoteSessionPromptNote(null)).toBe("");
    expect(remoteSessionPromptNote(undefined)).toBe("");
  });
  it("有的時候明講目標，並禁止再次 ssh", () => {
    const note = remoteSessionPromptNote("jamesju@192.168.1.80");
    expect(note).toContain("jamesju@192.168.1.80");
    expect(note).toMatch(/Do NOT run ssh/);
  });
});
