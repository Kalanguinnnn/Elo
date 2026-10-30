import { useState } from "react";
import { call, type Activity, type ProjectStatus } from "../api";
import { dateTime } from "../format";
import { Badge, Empty, Notice, useLoad } from "../ui";

interface QueueItem {
  id: number;
  project_id: string;
  kind: string;
  attempts: number;
  next_attempt_ms: number;
  last_error: string | null;
  failure_kind: string | null;
}

export default function ActivityPage({ projects }: { projects: ProjectStatus[] }) {
  const [pid, setPid] = useState("");
  const act = useLoad(() => call<Activity[]>("activity.list", { project_id: pid || null, limit: 300 }), [pid], 5000);
  const queue = useLoad(() => call<QueueItem[]>("queue.list"), [], 5000);
  const name = (id: string | null) => projects.find((p) => p.project.id === id)?.project.name ?? "";
  return (
    <div className="page">
      <div className="page-head">
        <h1>Atividade</h1>
        <select value={pid} onChange={(e) => setPid(e.target.value)}>
          <option value="">Todos os projetos</option>
          {projects.map((p) => <option key={p.project.id} value={p.project.id}>{p.project.name}</option>)}
        </select>
      </div>
      <section>
        <h2>Fila de sincronização</h2>
        <Notice error={queue.error} />
        {queue.data?.length === 0 ? (
          <Empty>Fila vazia.</Empty>
        ) : (
          <table className="table">
            <thead><tr><th>Projeto</th><th>Operação</th><th>Tentativas</th><th>Próxima tentativa</th><th>Último erro</th></tr></thead>
            <tbody>
              {queue.data?.map((q) => (
                <tr key={q.id}>
                  <td>{name(q.project_id)}</td>
                  <td>{q.kind}</td>
                  <td>{q.attempts}</td>
                  <td>{dateTime(q.next_attempt_ms)}</td>
                  <td className="small">{q.failure_kind && <Badge tone="warn">{q.failure_kind}</Badge>} {q.last_error}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      <section>
        <h2>Eventos</h2>
        <Notice error={act.error} />
        <div className="log">
          {act.data?.map((a) => (
            <div key={a.id} className={`log-row ${a.level}`}>
              <span className="muted small">{dateTime(a.ts_ms)}</span>
              <Badge tone={a.level === "error" ? "err" : a.level === "warn" ? "warn" : "neutral"}>{a.level}</Badge>
              {a.project_id && <span className="small">{name(a.project_id)}</span>}
              <span>{a.message}</span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
