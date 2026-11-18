import { describe, expect, it } from "vitest";
import { detectInputPrompt } from "./inputPrompt";

describe("detectInputPrompt", () => {
  it("winget 中文原文：[Y] 是  [N] 否: → Y/N", () => {
    expect(detectInputPrompt("[Y] 是  [N] 否: ")).toEqual({ kind: "yesno", options: ["Y", "N"] });
  });
  it("winget 英文：[Y] Yes  [N] No:", () => {
    expect(detectInputPrompt("[Y] Yes  [N] No: ")).toEqual({ kind: "yesno", options: ["Y", "N"] });
  });
  it("[Y/n]、[y/N]、(Y/N)", () => {
    expect(detectInputPrompt("Proceed with installation? [Y/n] ")).toEqual({ kind: "yesno", options: ["Y", "N"] });
    expect(detectInputPrompt("Continue? [y/N]")).toEqual({ kind: "yesno", options: ["Y", "N"] });
    expect(detectInputPrompt("Overwrite (Y/N)?")).toEqual({ kind: "yesno", options: ["Y", "N"] });
  });
  it("(yes/no) 要送完整單字", () => {
    expect(detectInputPrompt("Are you sure you want to continue connecting (yes/no)? ")).toEqual({
      kind: "yesno",
      options: ["yes", "no"],
    });
  });
  it("按任意鍵", () => {
    expect(detectInputPrompt("Press any key to continue . . .")).toEqual({ kind: "anykey", options: [""] });
    expect(detectInputPrompt("請按任意鍵繼續 . . .")).toEqual({ kind: "anykey", options: [""] });
  });
  it("密碼提示", () => {
    expect(detectInputPrompt("Password: ")).toEqual({ kind: "password", options: [] });
    expect(detectInputPrompt("[sudo] password for jim: ")).toEqual({ kind: "password", options: [] });
    expect(detectInputPrompt("Enter passphrase for key '/x/id_rsa': ")).toEqual({ kind: "password", options: [] });
    expect(detectInputPrompt("請輸入密碼：")).toEqual({ kind: "password", options: [] });
  });
  it("不該誤判：一般輸出、提示字元、提問不在行尾", () => {
    expect(detectInputPrompt("")).toBeNull();
    expect(detectInputPrompt("Downloading https://x.com/a.exe")).toBeNull();
    expect(detectInputPrompt("PS C:\\Users\\User> ")).toBeNull();
    expect(detectInputPrompt("The password was changed successfully.")).toBeNull();
    expect(detectInputPrompt("set [Y/n] defaults in config, then run again")).toBeNull();
    expect(detectInputPrompt("Password policy: 8 characters")).toBeNull();
  });
});
