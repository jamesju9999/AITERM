import { describe, expect, it } from "vitest";
import { redactSecrets, MASK } from "./redact";

const r = (s: string) => redactSecrets(s);

describe("redactSecrets – known token formats", () => {
  const cases: [string, string][] = [
    ["aws-access-key", "key AKIAIOSFODNN7EXAMPLE end"],
    ["github-token", "token ghp_abcdefghijklmnopqrstuvwxyz0123456789 ok"],
    ["github-token", "x github_pat_11ABCDEFG0abcdefghijklmnop_qrstuvwxyz0123456789ABCDEF y"],
    // 用執行期組出來的假 token：原始碼裡不放完整的 token 樣式，免得 GitHub 的 push protection 把測試資料當成真的密鑰擋下。
    ["slack-token", `slack ${["xoxb", "123456789012", "abcdefghijklmnop"].join("-")} ok`],
    ["anthropic-key", "ANTHROPIC sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"],
    ["api-key", "OPENAI sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD"],
    ["google-api-key", "k AIzaSyA-abcdefghijklmnopqrstuvwxyz01234 e"],
    ["npm-token", "npm npm_abcdefghijklmnopqrstuvwxyz0123456789 ok"],
    ["jwt", "t eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcDEF123-_xyz z"],
  ];
  it.each(cases)("masks a %s and keeps the surrounding text", (kind, input) => {
    const out = r(input);
    expect(out.count).toBe(1);
    expect(out.text).toContain(MASK);
    expect(out.kinds).toContain(kind);
    // 前後文字還在
    expect(out.text.startsWith(input.slice(0, 2))).toBe(true);
    expect(out.text).not.toMatch(/AKIAIOSFODNN7EXAMPLE|ghp_abcdef|xoxb-1234|sk-ant-api03-abc|sk-abcdefghijklmnopqrstuvwxyz|AIzaSyA-abc|npm_abcdef|eyJhbGciOiJIUzI1NiJ9\.eyJ/);
  });
});

describe("redactSecrets – private keys", () => {
  const body = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7\nabcdefghijklmnop0123456789";
  it("masks a whole multi-line private key block", () => {
    const input = `before\n-----BEGIN RSA PRIVATE KEY-----\n${body}\n-----END RSA PRIVATE KEY-----\nafter`;
    const out = r(input);
    expect(out.text).toBe(`before\n${MASK}\nafter`);
    expect(out.count).toBe(1);
    expect(out.kinds).toEqual(["private-key"]);
  });

  it("masks from BEGIN to the end of the text when the END line is missing (a cut-off screen)", () => {
    const out = r(`start\n-----BEGIN PRIVATE KEY-----\n${body}`);
    expect(out.text).toBe(`start\n${MASK}`);
    expect(out.text).not.toContain("MIIEvQ");
  });

  it("masks a key whose BEGIN line was cut off (only the END line is visible)", () => {
    const out = r(`${body}\n-----END RSA PRIVATE KEY-----\nafter`);
    expect(out.text).not.toContain("MIIEvQ");
    expect(out.text).toContain("after");
    expect(out.count).toBe(1);
    expect(out.kinds).toEqual(["private-key"]);
  });
});

describe("redactSecrets – HTTP credentials", () => {
  it("masks Bearer / Basic / Token values in an Authorization header", () => {
    expect(r("Authorization: Bearer abcdef1234567890XYZ").text).toBe(`Authorization: Bearer ${MASK}`);
    expect(r("authorization: basic dXNlcjpwYXNzd29yZA==").text).toBe(`authorization: basic ${MASK}`);
    expect(r('-H "Authorization: Token abcdef1234567890"').text).toContain(`Token ${MASK}`);
  });

  it("masking an already-masked header changes nothing and counts nothing", () => {
    const once = r("Authorization: Bearer abcdef1234567890XYZ").text;
    expect(r(once).count).toBe(0);
    expect(r(once).text).toBe(once);
  });

  it("does not mask a very short credential value in a header", () => {
    expect(r("Authorization: Bearer abc").count).toBe(0);
  });

  it("masks a bare long bearer token", () => {
    expect(r("curl -H 'x: Bearer abcdefghijklmnopqrstuvwx' u").text).toContain(`Bearer ${MASK}`);
  });

  it("does not touch the word Bearer in prose", () => {
    expect(r("the Bearer scheme is common").text).toBe("the Bearer scheme is common");
  });
});

describe("redactSecrets – credentials inside URLs", () => {
  it("masks only the password of scheme://user:password@host", () => {
    const out = r("git clone https://alice:s3cretPass@github.com/org/repo.git");
    expect(out.text).toBe(`git clone https://alice:${MASK}@github.com/org/repo.git`);
    expect(out.count).toBe(1);
  });

  it("masking an already-masked URL changes nothing and counts nothing", () => {
    const once = r("https://alice:s3cretPass@github.com/x").text;
    expect(r(once).count).toBe(0);
    expect(r(once).text).toBe(once);
  });

  it("leaves a URL without credentials, or with only a user, alone", () => {
    expect(r("https://github.com/org/repo.git").count).toBe(0);
    expect(r("ssh://git@github.com/org/repo.git").count).toBe(0);
  });
});

describe("redactSecrets – sensitive assignments", () => {
  it("masks the value but keeps the name", () => {
    expect(r("password=hunter2hunter2").text).toBe(`password=${MASK}`);
    expect(r("export API_KEY=abcd1234efgh").text).toBe(`export API_KEY=${MASK}`);
    expect(r("DB_PASSWORD: mysecretvalue").text).toBe(`DB_PASSWORD: ${MASK}`);
    expect(r("client_secret = 'abcdef123456'").text).toBe(`client_secret = ${MASK}`);
  });

  it("handles JSON style", () => {
    expect(r('{"password": "hunter2hunter2", "user": "bob"}').text).toBe(`{"password": ${MASK}, "user": "bob"}`);
    expect(r('{"api_key":"abcd1234efgh"}').text).toBe(`{"api_key":${MASK}}`);
  });

  it("matches names case-insensitively and inside longer names", () => {
    expect(r("GITHUB_TOKEN=abcdefgh1234").text).toBe(`GITHUB_TOKEN=${MASK}`);
    expect(r("mySecretKey = abcdef1234").count).toBe(1);
  });

  it("does not mask short, placeholder, boolean or empty values", () => {
    for (const v of ["token=true", "token=false", "token=null", "token=none", "token=undefined", "password=undefined", "password=", "password=abc", 'password=""',
      "token=<your-token>", "token=${TOKEN}", "token=$TOKEN", "password=your_password_here", "api_key=xxxxxxxx", "password=********"]) {
      expect(r(v).count, v).toBe(0);
    }
  });

  it("does not mask ordinary words that merely contain the letters", () => {
    expect(r("the author = someone").count).toBe(0); // author 不是 auth 賦值名稱
    expect(r("tokenizer: fastest-implementation").count).toBe(0);
  });
});

describe("redactSecrets – general behaviour", () => {
  it("returns the input unchanged with count 0 when nothing matches", () => {
    const text = "ls -la\nnpm run build\n✔ done in 3.2s";
    expect(r(text)).toEqual({ text, count: 0, kinds: [] });
  });

  it("counts every occurrence and lists each kind once", () => {
    const out = r("a ghp_abcdefghijklmnopqrstuvwxyz0123456789 b ghp_ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ c password=abcdef123456");
    expect(out.count).toBe(3);
    expect(out.kinds.sort()).toEqual(["github-token", "password"].sort());
  });

  it("is idempotent: masking masked text changes nothing and counts nothing", () => {
    const once = r("token ghp_abcdefghijklmnopqrstuvwxyz0123456789 and password=abcdef123456");
    const twice = r(once.text);
    expect(twice.text).toBe(once.text);
    expect(twice.count).toBe(0);
  });

  it("masks a secret even when it is the last thing on the line or the text", () => {
    expect(r("AKIAIOSFODNN7EXAMPLE").text).toBe(MASK);
  });

  it("does not leave a fragment when the token sits at a point where the text would later be cut", () => {
    const secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    const redacted = r("x ".repeat(25) + secret + " y".repeat(25)).text;
    // 先遮罩再截斷：不論從哪裡切，都不會出現 token 的任何一段
    for (let cut = 0; cut < redacted.length; cut += 7) {
      expect(redacted.slice(cut)).not.toMatch(/ghp_abc|klmnopqrstuvwxyz0123456789/);
    }
  });
});
