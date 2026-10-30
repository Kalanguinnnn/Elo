// Pure helpers (unit-tested in format.test.ts).

export function relTime(ms: number | null | undefined, now = Date.now()): string {
  if (!ms) return "nunca";
  const s = Math.round((now - ms) / 1000);
  if (s < 0) return "agora";
  if (s < 45) return "agora há pouco";
  if (s < 3600) return `há ${Math.round(s / 60)} min`;
  if (s < 86400) return `há ${Math.round(s / 3600)} h`;
  return `há ${Math.round(s / 86400)} d`;
}

export function dateTime(ms: number): string {
  return new Date(ms).toLocaleString("pt-BR");
}

export function shortBranch(b: string | null | undefined): string {
  if (!b) return "(HEAD destacado)";
  return b.replace(/^refs\/heads\//, "");
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}

const PUSH_LABELS: Record<string, string> = {
  pending: "aguardando envio",
  pushed: "enviado",
  needs_review: "aguarda revisão de segredos",
  quarantined: "bloqueado (credencial)",
  remote: "recebido",
};

export function pushLabel(state: string): string {
  return PUSH_LABELS[state] ?? state;
}

const TRIGGERS: Record<string, string> = {
  manual: "manual",
  timer: "periódico",
  watcher: "alterações detectadas",
  shutdown: "ao encerrar",
  pre_apply: "segurança (antes de aplicar)",
  merge: "mesclagem",
  pre_restore: "segurança (antes de restaurar)",
};

export function triggerLabel(t: string): string {
  return TRIGGERS[t] ?? t;
}

/** Strip the "[code] " prefix the IPC layer adds to errors. */
export function errorText(e: unknown): string {
  const s = typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
  return s.replace(/^\[[a-z_]+\]\s*/, "");
}

export function utf8ToBase64(text: string): string {
  const bytesArr = new TextEncoder().encode(text);
  let bin = "";
  bytesArr.forEach((b) => (bin += String.fromCharCode(b)));
  return btoa(bin);
}
