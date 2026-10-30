import { useState } from "react";
import type { ProjectStatus } from "../api";
import type { Nav } from "../App";
import { relTime } from "../format";
import { Badge, Empty } from "../ui";
import AddProject from "./AddProject";
import { useGithub } from "./GitHubConnect";
import { IconRepo } from "../icons";

export function statusBadges(s: ProjectStatus) {
  const b: React.ReactNode[] = [];
  if (!s.available) b.push(<Badge key="na" tone="err">pasta indisponível</Badge>);
  if (s.project.paused) b.push(<Badge key="p">pausado</Badge>);
  if (s.open_conflicts) b.push(<Badge key="c" tone="err">{s.open_conflicts} conflito(s)</Badge>);
  if (s.incoming) b.push(<Badge key="i" tone="info">{s.incoming} atualização(ões)</Badge>);
  if (s.quarantined) b.push(<Badge key="q" tone="err">credencial bloqueada</Badge>);
  if (s.needs_review) b.push(<Badge key="r" tone="warn">revisar segredos</Badge>);
  if (s.queue_failure_kind === "offline") b.push(<Badge key="o" tone="warn">offline — na fila</Badge>);
  else if (s.queue_error) b.push(<Badge key="e" tone="err">erro de envio</Badge>);
  if (s.interrupted_ops) b.push(<Badge key="x" tone="warn">operação interrompida</Badge>);
  if (s.project.trusted) b.push(<Badge key="t" tone="ok">confiável</Badge>);
  if (!b.length && s.available) b.push(<Badge key="ok" tone="ok">Tudo em dia</Badge>);
  return b;
}

export default function Projects({ nav, list, reload }: { nav: Nav; list: ProjectStatus[]; reload: () => void }) {
  const [adding, setAdding] = useState(false);
  const gh = useGithub();
  return (
    <div className="page">
      <div className="page-head">
        <h1>Projetos</h1>
        <button className="primary" onClick={() => setAdding(!adding)}>{adding ? "Cancelar" : "Adicionar projeto"}</button>
      </div>
      {adding && (
        <AddProject
          githubConnected={!!gh.data?.connected}
          onAdded={(p) => {
            setAdding(false);
            reload();
            nav({ page: "project", id: p.id });
          }}
        />
      )}
      {list.length === 0 && !adding && <Empty>Nenhum projeto ainda.</Empty>}
      <div className="list">
        {list.map((s) => (
          <button key={s.project.id} className="list-item" onClick={() => nav({ page: "project", id: s.project.id })}>
            <div className="li-title">
              <span className="project-glyph"><IconRepo /></span>
              <div className="li-main">
                <strong>{s.project.name}</strong>
                <span className="muted mono small">{s.project.path}</span>
              </div>
            </div>
            <div className="li-meta">
              <span className="muted small">
                {s.branch ?? "—"} · {s.uncommitted_changes ?? 0} alteração(ões) locais · checkpoint {relTime(s.project.last_checkpoint_ms)}
              </span>
              <div className="badges">{statusBadges(s)}</div>
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}
