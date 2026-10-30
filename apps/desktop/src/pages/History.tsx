import { useState } from "react";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { call, type ApplyOutcome, type Checkpoint, type DiffEntry, type ProjectStatus } from "../api";
import type { Nav } from "../App";
import { dateTime, pushLabel, shortBranch, triggerLabel } from "../format";
import { Badge, Empty, Notice, useAction, useLoad } from "../ui";
import { outcomeText } from "./ProjectDetail";

interface RestoreReport {
  target: string;
  files_written: number;
  skipped: string[];
  integrity_failures: string[];
  verified: number;
}

function Detail({ projectId, c, nav }: { projectId: string; c: Checkpoint; nav: Nav }) {
  const files = useLoad(() => call<DiffEntry[]>("compare", { project_id: projectId, from: { kind: "current" }, to: { kind: "checkpoint", id: c.id } }), [c.id]);
  const [sel, setSel] = useState<Record<string, boolean>>({});
  const act = useAction();

  const restoreFiles = async () => {
    const paths = Object.keys(sel).filter((k) => sel[k]);
    const ok = await ask(`Restaurar ${paths.length} arquivo(s) para a versão de ${dateTime(c.created_ms)}? O estado atual será salvo antes em um checkpoint de segurança.`, { title: "Restaurar arquivos", kind: "warning" });
    if (!ok) return;
    act.run(() => call<ApplyOutcome>("restore.files", { project_id: projectId, checkpoint_id: c.id, paths }), (o) => { files.reload(); setSel({}); return outcomeText(o); });
  };
  const restoreDir = async () => {
    const dir = await open({ directory: true, title: "Pasta vazia de destino (fora do projeto)" });
    if (typeof dir !== "string") return;
    act.run(
      () => call<RestoreReport>("restore.dir", { project_id: projectId, checkpoint_id: c.id, target: dir }),
      (r) =>
        r.integrity_failures.length
          ? `Recuperado em ${r.target}, mas ${r.integrity_failures.length} arquivo(s) falharam na verificação: ${r.integrity_failures.join(", ")}`
          : `Recuperado em ${r.target}: ${r.files_written} arquivo(s), ${r.verified} verificado(s) por hash${r.skipped.length ? `, ${r.skipped.length} ignorado(s)` : ""}.`,
    );
  };

  return (
    <div className="card stack">
      <div className="row between">
        <div>
          <strong>{dateTime(c.created_ms)}</strong> · {c.device_name} · {shortBranch(c.branch)} · {triggerLabel(c.trigger)}
          <div className="muted small mono">{c.id} · HEAD {c.head_oid?.slice(0, 10) ?? "—"}</div>
        </div>
        <div className="row">
          <button onClick={() => nav({ page: "compare", id: projectId, to: c.id })}>Comparar</button>
          <button onClick={restoreDir} disabled={act.busy}>Recuperar em outra pasta…</button>
        </div>
      </div>
      <div className="small muted">Diferenças entre o diretório atual e este checkpoint (marque para restaurar):</div>
      {files.error && <Notice error={files.error} />}
      {files.data?.length === 0 && <Empty>Idêntico ao diretório atual.</Empty>}
      <div className="filelist">
        {files.data?.map((f) => (
          <label key={f.path} className="file">
            <input type="checkbox" checked={!!sel[f.path]} onChange={(e) => setSel({ ...sel, [f.path]: e.target.checked })} />
            <span className={`st st-${f.status}`}>{f.status}</span>
            <span className="mono small">{f.old_path ? `${f.old_path} → ` : ""}{f.path}</span>
          </label>
        ))}
      </div>
      <div className="row">
        <button className="primary" onClick={restoreFiles} disabled={act.busy || !Object.values(sel).some(Boolean)}>Restaurar selecionados</button>
      </div>
      <Notice error={act.error} message={act.message} />
    </div>
  );
}

export default function History({ nav, id }: { nav: Nav; id: string }) {
  const st = useLoad(() => call<ProjectStatus>("project.status", { project_id: id }), [id]);
  const hist = useLoad(() => call<Checkpoint[]>("history.list", { project_id: id, limit: 500 }), [id], 15000);
  const [device, setDevice] = useState("");
  const [open_, setOpen] = useState<string | null>(null);
  const devices = Array.from(new Set((hist.data ?? []).map((c) => c.device_name)));
  const rows = (hist.data ?? []).filter((c) => !device || c.device_name === device);
  return (
    <div className="page">
      <button className="link" onClick={() => nav({ page: "project", id })}>← {st.data?.project.name ?? "Projeto"}</button>
      <div className="page-head">
        <h1>Histórico</h1>
        <select value={device} onChange={(e) => setDevice(e.target.value)}>
          <option value="">Todos os dispositivos</option>
          {devices.map((d) => <option key={d}>{d}</option>)}
        </select>
      </div>
      <Notice error={hist.error} />
      {rows.length === 0 && <Empty>Nenhum checkpoint.</Empty>}
      <div className="list">
        {rows.map((c) => (
          <div key={c.id}>
            <button className={`list-item ${open_ === c.id ? "sel" : ""}`} onClick={() => setOpen(open_ === c.id ? null : c.id)}>
              <div className="li-main">
                <strong>{dateTime(c.created_ms)}</strong>
                <span className="muted small">{c.summary || "—"}</span>
              </div>
              <div className="li-meta">
                <span className="small">{c.device_name} · {shortBranch(c.branch)} · {triggerLabel(c.trigger)}</span>
                <div className="badges">
                  {st.data?.project.lineage_head === c.id && <Badge tone="info">estado atual</Badge>}
                  {c.applied_ms && <Badge tone="ok">aplicado aqui</Badge>}
                  <Badge tone={c.push_state === "quarantined" ? "err" : c.push_state === "needs_review" ? "warn" : "neutral"}>{pushLabel(c.push_state)}</Badge>
                </div>
              </div>
            </button>
            {open_ === c.id && <Detail projectId={id} c={c} nav={nav} />}
          </div>
        ))}
      </div>
    </div>
  );
}
