import { describe, expect, it } from "vitest";
import { readPref, setPref } from "./motion";

function memStore(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

describe("motion preference", () => {
  it("animations are on unless performance mode was chosen", () => {
    expect(readPref(memStore())).toBe("full");
    expect(readPref(memStore({ "elo.motion": "off" }))).toBe("off");
    expect(readPref(memStore({ "elo.motion": "weird" }))).toBe("full");
  });
  it("survives missing or broken storage", () => {
    expect(readPref(null)).toBe("full");
    expect(readPref({ ...memStore(), getItem: () => { throw new Error("blocked"); } })).toBe("full");
  });
  it("persists the choice and marks the document", () => {
    const html = { dataset: {} as Record<string, string> };
    (globalThis as { document?: unknown }).document = { documentElement: html };
    const s = memStore();
    setPref("off", s);
    expect(readPref(s)).toBe("off");
    expect(html.dataset.motion).toBe("off");
    setPref("full", s);
    expect(readPref(s)).toBe("full");
    expect(html.dataset.motion).toBe("full");
  });
});
