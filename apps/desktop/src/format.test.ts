import { describe, expect, it } from "vitest";
import { bytes, errorText, relTime, shortBranch, utf8ToBase64 } from "./format";
import { sideBySide } from "./diff";

describe("format", () => {
  it("relative time", () => {
    const now = 1_000_000_000;
    expect(relTime(null, now)).toBe("nunca");
    expect(relTime(now - 10_000, now)).toBe("agora há pouco");
    expect(relTime(now - 600_000, now)).toBe("há 10 min");
    expect(relTime(now - 7_200_000, now)).toBe("há 2 h");
  });
  it("branches, sizes, errors", () => {
    expect(shortBranch("refs/heads/feature/x")).toBe("feature/x");
    expect(shortBranch(null)).toBe("(HEAD destacado)");
    expect(bytes(2048)).toBe("2.0 KB");
    expect(errorText("[conflict] arquivo mudou")).toBe("arquivo mudou");
  });
  it("base64 matches the Rust decoder's alphabet for UTF-8", () => {
    expect(utf8ToBase64("foobar")).toBe("Zm9vYmFy");
    expect(utf8ToBase64("ç")).toBe("w6c=");
  });
});

describe("side-by-side diff", () => {
  it("aligns unchanged, removed and added lines", () => {
    const rows = sideBySide("a\nb\nc\n", "a\nB\nc\nd\n");
    expect(rows.map((r) => r.kind)).toEqual(["same", "change", "same", "add"]);
    expect(rows[1]).toMatchObject({ left: "b", right: "B" });
    expect(rows[3]).toMatchObject({ left: null, right: "d" });
  });
});
