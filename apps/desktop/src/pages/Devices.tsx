import { useState } from "react";
import { call, type Device } from "../api";
import { dateTime, relTime } from "../format";
import { Badge, Notice, useAction, useLoad } from "../ui";

export default function Devices() {
  const list = useLoad(() => call<Device[]>("devices.list"), [], 15000);
  const [name, setName] = useState<string | null>(null);
  const act = useAction();
  const local = list.data?.find((d) => d.is_local);
  return (
    <div className="page">
      <h1>Dispositivos</h1>
      <p className="muted">Computadores que já criaram checkpoints nos seus projetos. Cada um tem um ID persistente gerado localmente.</p>
      {local && (
        <div className="card stack">
          <label className="field">
            <span>Nome deste computador</span>
            <div className="row">
              <input value={name ?? local.name} onChange={(e) => setName(e.target.value)} />
              <button disabled={act.busy || !name} onClick={() => act.run(async () => { await call("device.rename", { name }); setName(null); list.reload(); }, () => "Nome atualizado (vale para os próximos checkpoints).")}>Renomear</button>
            </div>
          </label>
          <div className="muted small mono">{local.id}</div>
          <Notice error={act.error} message={act.message} />
        </div>
      )}
      <Notice error={list.error} />
      <table className="table">
        <thead><tr><th>Nome</th><th>ID</th><th>Visto pela primeira vez</th><th>Última atividade</th></tr></thead>
        <tbody>
          {list.data?.map((d) => (
            <tr key={d.id}>
              <td>{d.name} {d.is_local && <Badge tone="info">este</Badge>}</td>
              <td className="mono small">{d.id.slice(0, 8)}</td>
              <td>{dateTime(d.first_seen_ms)}</td>
              <td>{relTime(d.last_seen_ms)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
