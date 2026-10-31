import type { ProjectStatus } from "./api";

/** One-line health summary for the sidebar, from real project status. */
export function health(list: ProjectStatus[] | null, error: string | null): { tone: "ok" | "warn" | "err" | "info" | "idle"; text: string } {
  if (error && !list) return { tone: "err", text: "Agent indisponível" };
  if (!list) return { tone: "idle", text: "Conectando…" };
  if (!list.length) return { tone: "idle", text: "Nenhum projeto" };
  if (list.some((p) => p.open_conflicts || p.quarantined || p.unsafe_config)) return { tone: "err", text: "Requer sua atenção" };
  if (list.some((p) => p.queue_failure_kind === "offline")) return { tone: "warn", text: "Offline — na fila" };
  if (list.some((p) => p.incoming)) return { tone: "info", text: "Atualizações disponíveis" };
  if (list.some((p) => p.pending_push || p.needs_review)) return { tone: "warn", text: "Envio pendente" };
  return { tone: "ok", text: "Tudo em dia" };
}
