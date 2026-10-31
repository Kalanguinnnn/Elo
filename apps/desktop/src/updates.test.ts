import { describe, expect, it } from "vitest";
import { autoCheckEnabled, setAutoCheck, updateErrorText } from "./updates";

function memStore(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

describe("self-update", () => {
  it("checks automatically unless turned off on this computer", () => {
    expect(autoCheckEnabled(memStore())).toBe(true);
    expect(autoCheckEnabled(null)).toBe(true);
    const s = memStore();
    setAutoCheck(false, s);
    expect(autoCheckEnabled(s)).toBe(false);
    setAutoCheck(true, s);
    expect(autoCheckEnabled(s)).toBe(true);
  });
  it("explains failures in plain words", () => {
    expect(updateErrorText(new Error("Could not fetch a valid release JSON from the remote"))).toMatch(/Nenhuma versão publicada/);
    expect(updateErrorText("signature verification failed")).toMatch(/assinatura/);
    expect(updateErrorText(new Error("error sending request: dns error"))).toMatch(/Sem conexão/);
    expect(updateErrorText("algo inesperado")).toBe("algo inesperado");
  });
});
