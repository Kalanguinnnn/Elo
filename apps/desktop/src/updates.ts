import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

/**
 * Self-update from the published GitHub releases. The plugin verifies the
 * package signature against the public key built into the app before
 * installing; drafts are never offered (GitHub's "latest" only sees published
 * releases). The Agent is stopped right before installing so the installer
 * can replace its executable; queued work resumes in the new version.
 */
export type UpdatePhase = "idle" | "checking" | "none" | "available" | "downloading" | "installing" | "error";

export interface UpdateState {
  phase: UpdatePhase;
  version?: string;
  notes?: string;
  /** 0..1 while downloading, when the size is known. */
  progress?: number | null;
  error?: string;
}

export const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;
const AUTO_KEY = "elo.updates.auto";

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

/** Automatic checks are on unless the user turned them off on this computer. */
export function autoCheckEnabled(s: Store | null = store()): boolean {
  try {
    return s?.getItem(AUTO_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setAutoCheck(on: boolean, s: Store | null = store()) {
  try {
    s?.setItem(AUTO_KEY, on ? "on" : "off");
  } catch {
    /* the choice lasts for this window only */
  }
  emit();
}

/** Readable message for a failed check or install. */
export function updateErrorText(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e);
  if (/release json|404|not found/i.test(msg)) return "Nenhuma versão publicada foi encontrada no GitHub.";
  if (/signature/i.test(msg)) return "A assinatura do pacote não confere; a atualização foi recusada por segurança.";
  if (/network|connect|dns|timed? ?out|offline/i.test(msg)) return "Sem conexão com o GitHub para procurar atualizações.";
  return msg;
}

let state: UpdateState = { phase: "idle" };
let pending: Update | null = null;
const listeners = new Set<() => void>();

function set(next: UpdateState) {
  state = next;
  emit();
}

function emit() {
  listeners.forEach((l) => l());
}

function inApp(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Look for a newer published version. Quiet failures for automatic checks. */
export async function checkForUpdate(manual: boolean) {
  if (!inApp() || state.phase === "checking" || state.phase === "downloading" || state.phase === "installing") return;
  set({ phase: "checking" });
  try {
    pending = await check();
    set(pending ? { phase: "available", version: pending.version, notes: pending.body ?? undefined } : { phase: "none" });
  } catch (e) {
    pending = null;
    set(manual ? { phase: "error", error: updateErrorText(e) } : { phase: "idle" });
  }
}

/** Download, verify, stop the Agent, install and restart. */
export async function installUpdate() {
  const u = pending;
  if (!u) return;
  let total = 0;
  let done = 0;
  set({ ...state, phase: "downloading", progress: null });
  try {
    await u.download((ev) => {
      if (ev.event === "Started") total = ev.data.contentLength ?? 0;
      if (ev.event === "Progress") {
        done += ev.data.chunkLength;
        set({ ...state, progress: total ? Math.min(done / total, 1) : null });
      }
    });
  } catch (e) {
    set({ phase: "error", version: u.version, error: updateErrorText(e) });
    return;
  }
  set({ ...state, phase: "installing", progress: 1 });
  try {
    await invoke("stop_agent_for_update");
    await u.install(); // on Windows the installer takes over and restarts the app
    await relaunch();
  } catch (e) {
    await invoke("resume_after_failed_update").catch(() => undefined);
    set({ phase: "error", version: u.version, error: updateErrorText(e) });
  }
}

export function useUpdates() {
  const [, tick] = useState(0);
  useEffect(() => {
    const l = () => tick((n) => n + 1);
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  }, []);
  return { ...state, auto: autoCheckEnabled(), check: checkForUpdate, install: installUpdate, setAuto: setAutoCheck };
}

/** Check shortly after start and then periodically, if enabled. */
export function useAutoUpdateCheck() {
  useEffect(() => {
    const run = () => {
      if (autoCheckEnabled()) checkForUpdate(false);
    };
    const first = setTimeout(run, 10_000);
    const every = setInterval(run, CHECK_EVERY_MS);
    return () => {
      clearTimeout(first);
      clearInterval(every);
    };
  }, []);
}
