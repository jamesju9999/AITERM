#!/usr/bin/env python3
"""演練用的假 claude：只模仿 AITerm 問題收集會碰到的行為，不花任何額度。

- 讀 `--settings <檔>` 裡的 HTTP hook 網址（跟真 claude 一樣）。
- 收到提問（輸入停頓 1.5 秒才算一則，因為多行貼上在終端機裡是好幾行）→
  打 UserPromptSubmit hook → 工作 → 寫 transcript → 打 Stop hook。
- 提問含 `HANG` 就永遠不回；含 `BUGFIX` 會在目前資料夾建立 fixed.txt（假裝修了程式）。
"""
import json, os, select, sys, time, urllib.request

log = open(os.environ["FAKE_CLAUDE_LOG"], "a", buffering=1)
def note(*a): log.write(" ".join(str(x) for x in a) + "\n")

args = sys.argv[1:]
settings = None
if "--settings" in args:
    settings = json.load(open(args[args.index("--settings") + 1]))
note("ARGS", "has_settings" if settings else "no_settings")

def hook(name, payload):
    if not settings:
        return
    url = settings["hooks"][name][0]["hooks"][0]["url"]
    req = urllib.request.Request(url, data=json.dumps(payload).encode(), headers={"Content-Type": "application/json"})
    try:
        urllib.request.urlopen(req, timeout=5).read()
        note("HOOK", name, "ok")
    except Exception as e:
        note("HOOK", name, "FAIL", e)

print("READY", flush=True)
buf = b""
while True:
    r, _, _ = select.select([0], [], [], 1.5)
    if r:
        chunk = os.read(0, 65536)
        if not chunk:
            break
        buf += chunk
        continue
    if not buf.strip():
        continue
    prompt = buf.decode("utf-8", "replace").replace("\r", "\n").strip()
    buf = b""
    note("PROMPT_BEGIN"); note(prompt); note("PROMPT_END")
    hook("UserPromptSubmit", {"prompt": prompt})
    if "HANG" in prompt:
        note("HANGING"); time.sleep(3600)
    paths = [l[2:].strip() for l in prompt.splitlines() if l.startswith("- /")]
    for p in paths:
        note("IMG", "ok" if os.path.exists(p) else "MISSING", os.path.getsize(p) if os.path.exists(p) else 0, p)
    if "BUGFIX" in prompt:
        open("fixed.txt", "w").write("fixed\n")
    answer = "已處理完成。問題出在 /Users/someone/secret-project/src/app.py，金鑰 sk-abcdefghijklmnop12345 不要外流。"
    tp = os.path.join(os.path.dirname(os.environ["FAKE_CLAUDE_LOG"]), "transcript.jsonl")
    with open(tp, "w") as f:
        f.write(json.dumps({"type": "user", "message": {"content": prompt}}) + "\n")
        f.write(json.dumps({"type": "assistant", "message": {"content": [{"type": "text", "text": answer}]}}) + "\n")
    time.sleep(0.5)
    hook("Stop", {"transcript_path": tp})
    print("DONE", flush=True)
