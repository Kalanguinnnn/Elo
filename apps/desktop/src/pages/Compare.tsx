import { useState } from "react";
import { call, type Checkpoint, type DiffEntry, type FileView, type Side } from "../api";
import type { Nav } from "../App";
import { dateTime } from "../format";
import { DiffView, Empty, Notice, useLoad } from "../ui";

function sideFrom(v: string): Side {
  if (v === "current") return { kind: "current" };
  if (v === "head") return { kind: "head" };
  return { kind: "checkpoint", id: v };
}

function label(v: string, list: Checkpoint[]): string {
  if (v === "current") return "Diretório atual";
  if (v === "head") return "Último commit (HEAD)";
  const c = list.find((x) => x.id === v);
  return c ? `${dateTime(c.created_ms)} · ${c.device_name}` : v;
}

export default function Compare({ nav, id, from, to }: { nav: Nav; id: string; from?: string; to?: string }) {
  const hist = useLoad(() => call<Checkpoint[]>("history.list", { project_id: id, limit: 500 }), [id]);
  const [a, setA] = useState(from ?? "head");
  const [b, setB] = useState(to ?? "current");
  const [file, setFile] = useState<string | null>(null);
  const files = useLoad(() => call<DiffEntry[]>("compare", { project_id: id, from: sideFrom(a), to: sideFrom(b) }), [a, b]);
  const entry = files.data?.find((f) => f.path === file);
  const views = useLoad(
    async () =>
      file && entry
        ? Promise.all([
            call<FileView>("file.view", { project_id: id, side: sideFrom(a), path: entry.old_path ?? file }),
            call<FileView>("file.view", { project_id: id, side: sideFrom(b), path: file }),
          ])
        : null,
    [a, b, file],
  );
  const list = hist.data ?? [];
  const opts = (
    <>
      <option value="current">Diretório atual</option>
      <option value="head">Último commit (HEAD)</option>
      {list.map((c) => (
        <option key={c.id} value={c.id}>{label(c.id, list)}</option>
      ))}
    </>
  );
  return (
    <div className="page wide">
      <button className="link" onClick={() => nav({ page: "project", id })}>← Projeto</button>
      <h1>Comparar versões</h1>
      <div className="row">
        <select value={a} onChange={(e) => { setA(e.target.value); setFile(null); }}>{opts}</select>
        <span>→</span>
        <select value={b} onChange={(e) => { setB(e.target.value); setFile(null); }}>{opts}</select>
      </div>
      <Notice error={files.error ?? views.error} />
      <div className="compare">
        <div className="filelist side">
          {files.data?.length === 0 && <Empty>Sem diferenças.</Empty>}
          {files.data?.map((f) => (
            <button key={f.path} className={`file ${file === f.path ? "sel" : ""}`} onClick={() => setFile(f.path)}>
              <span className={`st st-${f.status}`}>{f.status}</span>
              <span className="mono small">{f.path}</span>
            </button>
          ))}
        </div>
        <div className="grow">
          {views.data ? <DiffView left={views.data[0]} right={views.data[1]} leftTitle={label(a, list)} rightTitle={label(b, list)} /> : <Empty>Selecione um arquivo.</Empty>}
        </div>
      </div>
    </div>
  );
}
