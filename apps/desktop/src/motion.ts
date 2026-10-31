import { useEffect, useState } from "react";

/**
 * Animation preference for this computer's window. Purely visual, so it lives in
 * the webview's storage instead of the engine settings. Animations are on by
 * default (Windows with "Efeitos de animação" off also reports reduced motion,
 * which would hide them for most users); "Modo desempenho" turns every animation
 * and transition off via `data-motion="off"` on <html>.
 */
export type MotionPref = "full" | "off";

const KEY = "elo.motion";
const listeners = new Set<() => void>();

interface Store {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

function store(): Store | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function readPref(s: Store | null = store()): MotionPref {
  try {
    return s?.getItem(KEY) === "off" ? "off" : "full";
  } catch {
    return "full";
  }
}

export function applyMotion(pref: MotionPref = readPref()) {
  document.documentElement.dataset.motion = pref;
}

export function setPref(pref: MotionPref, s: Store | null = store()) {
  try {
    s?.setItem(KEY, pref);
  } catch {
    /* storage unavailable: the choice still applies to this window */
  }
  applyMotion(pref);
  listeners.forEach((l) => l());
}

/** Current preference plus a setter for the "Modo desempenho" switch. */
export function useMotion() {
  const [pref, setLocal] = useState<MotionPref>(readPref);
  useEffect(() => {
    const sync = () => setLocal(readPref());
    listeners.add(sync);
    return () => { listeners.delete(sync); };
  }, []);
  return { performance: pref === "off", set: setPref };
}
