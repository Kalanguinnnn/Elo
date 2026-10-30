import { useEffect, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { call, type ApplyOutcome, type ConflictDetail, type ConflictRow } from "../api";
import type { Nav } from "../App";
import { dateTime, utf8ToBase64 } from "../format";
import { DiffView, Empty, Notice, useAction, useLoad } from "../ui";
import { outcomeText } from "./ProjectDetail";

type Choice = "ours" | "theirs" | "both" | "custom";

const REASONS: Record<string, string> = {
  both_modified: "modificado nos dois computadores",
  local_untracked: "existe localmente como arquivo não rastreado/ignorado",
  case_collision: "colide com outro arquivo (maiúsculas/minúsculas)",
};

function Resolver({ conflictId, onDone }: { conflictId: string; onDone: () => void }) {
  const d = useLoad(() => call<ConflictDetail>("conflict.detail", { conflict_id: conflictId }), [conflictId]);
  const [choice, setChoice] = useState<Record<string, Choice>>({});
  const [custom, setCustom] = useState<Record<string, string>>({});
  const act = useAction();
  useEffect(() => setChoice({}), [conflictId]);
  if (!d.data) return <Notice error={d.error} />;
  const det = d.data;
  const complete = det.files.every((f) => choice[f.path]);

  const resolve = () =>
    act.run(async () => {
      const resolutions: Record<string, unknown> = {};
      for (const f of det.files) {
        const c = choice[f.path];
        resolutions[f.path] = c === "custom" ? { choice: "custom", content_b64: utf8ToBase64(custom[f.path] ?? "") } : { choice: c };
      }
      const o = await call<ApplyOutcome>("conflict.resolve", { conflict_id: conflictId, resolutions });
      onDone();
      return o;
    }, outcomeText);

  const abort = async () => {
    if (!(await ask("Descartar este conflito? Nenhum arquivo foi alterado; a atualização poderá ser aplicada de novo depois.", { title: "Descartar", kind: "warning" }))) return;
    act.run(async () => { await call("conflict.abort", { conflict_id: conflictId }); onDone(); });
  };

  return (
    <div className="stack">
      <p className="small">
        Atualização de <strong>{det.theirs_device}</strong>. Nada foi alterado ainda. A versão não escolhida é preservada em
        <code> .git/continuity/preserved</code> e nos checkpoints (estado local salvo em {det.conflict.safety_checkpoint}).
      </p>
      {det.files.map((f) => (
        <div key={f.path} className="card stack">
          <div className="row between">
            <div>
              <strong className="mono">{f.path}</strong> <span className="muted small">— {REASONS[f.reason] ?? f.reason}</span>
            </div>
            <div className="seg">
              {(["ours", "theirs", "both", "custom"] as Choice[]).map((c) => (
                <button
                  key={c}
                  className={choice[f.path] === c ? "active" : ""}
                  disabled={c === "custom" && (f.ours.binary || f.theirs.binary)}
                  onClick={() => {
                    setChoice({ ...choice, [f.path]: c });
                    if (c === "custom" && custom[f.path] === undefined) setCustom({ ...custom, [f.path]: f.ours.text ?? f.theirs.text ?? "" });
                  }}
                >
                  {{ ours: "Manter este computador", theirs: `Usar ${det.theirs_device}`, both: "Manter ambos", custom: "Editar" }[c]}
                </button>
              ))}
            </div>
          </div>
          <DiffView left={f.ours} right={f.theirs} leftTitle="Este computador" rightTitle={det.theirs_device} />
          {choice[f.path] === "custom" && (
            <textarea className="code editor" value={custom[f.path] ?? ""} onChange={(e) => setCustom({ ...custom, [f.path]: e.target.value })} />
          )}
        </div>
      ))}
      <div className="row">
        <button className="primary" disabled={!complete || act.busy} onClick={resolve}>Aplicar resoluções</button>
        <button onClick={abort} disabled={act.busy}>Descartar conflito</button>
      </div>
      <Notice error={act.error} message={act.message} />
    </div>
  );
}

export default function Conflicts({ projectId, conflictId, onChanged }: { nav: Nav; projectId?: string; conflictId?: string; onChanged: () => void }) {
  const list = useLoad(() => call<ConflictRow[]>("conflicts.list", { project_id: projectId ?? null }), [projectId], 10000);
  const [sel, setSel] = useState<string | null>(conflictId ?? null);
  const current = sel ?? list.data?.[0]?.id ?? null;
  return (
    <div className="page wide">
      <h1>Conflitos</h1>
      <Notice error={list.error} />
      {list.data?.length === 0 && <Empty>Nenhum conflito pendente.</Empty>}
      {(list.data?.length ?? 0) > 1 && (
        <select value={current ?? ""} onChange={(e) => setSel(e.target.value)}>
          {list.data!.map((c) => <option key={c.id} value={c.id}>{dateTime(c.created_ms)} · {c.checkpoint_id}</option>)}
        </select>
      )}
      {current && <Resolver conflictId={current} onDone={() => { setSel(null); list.reload(); onChanged(); }} />}
    </div>
  );
}
