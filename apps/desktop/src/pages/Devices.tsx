import { useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { call, type Device } from "../api";
import { dateTime, relTime } from "../format";
import { Badge, Notice, Toggle, useAction, useLoad } from "../ui";

export default function Devices() {
  const list = useLoad(() => call<Device[]>("devices.list"), [], 15000);
  const [name, setName] = useState<string | null>(null);
  const act = useAction();
  const local = list.data?.find((d) => d.is_local);
  const others = list.data?.filter((d) => !d.is_local) ?? [];

  const approve = async (d: Device, v: boolean) => {
    if (v) {
      const ok = await ask(
        `Abra o Elo no computador "${d.name}", vá em Dispositivos e confira se a impressão digital dele é exatamente:\n\n${d.fingerprint}\n\n` +
          "Só aprove se for igual. Checkpoints assinados por este computador poderão ser aplicados automaticamente nos projetos marcados como confiáveis.",
        { title: "Aprovar dispositivo", kind: "warning" },
      );
      if (!ok) return;
    }
    act.run(async () => {
      await call("device.approve", { device_id: d.id, approved: v });
      list.reload();
    }, () => (v ? `"${d.name}" aprovado para aplicação automática.` : `"${d.name}" não aplica mais automaticamente.`));
  };

  const resetKey = async (d: Device) => {
    const ok = await ask(
      `Esquecer a chave de "${d.name}"? Use isto só se esse computador realmente gerou uma chave nova (por exemplo, após reinstalar o sistema). ` +
        "A aprovação será revogada e você precisará conferir a nova impressão digital.",
      { title: "Redefinir chave", kind: "warning" },
    );
    if (!ok) return;
    act.run(async () => {
      await call("device.reset_key", { device_id: d.id });
      list.reload();
    }, () => `Chave de "${d.name}" esquecida. A próxima atualização assinada fixará a nova chave.`);
  };

  return (
    <div className="page">
      <h1>Dispositivos</h1>
      <p className="muted">
        Cada computador assina os próprios checkpoints. Atualizações só são aplicadas automaticamente quando vêm de um computador que você
        aprovou aqui, depois de conferir a impressão digital.
      </p>
      {local && (
        <div className="card stack">
          <label className="field">
            <span>Nome deste computador</span>
            <div className="row">
              <input value={name ?? local.name} onChange={(e) => setName(e.target.value)} />
              <button disabled={act.busy || !name} onClick={() => act.run(async () => { await call("device.rename", { name }); setName(null); list.reload(); }, () => "Nome atualizado (vale para os próximos checkpoints).")}>Renomear</button>
            </div>
          </label>
          <div className="field">
            <span>Impressão digital deste computador</span>
            {local.fingerprint ? (
              <div className="fingerprint">{local.fingerprint}</div>
            ) : (
              <div className="notice">Assinatura indisponível: o cofre de credenciais do sistema não pôde ser usado.</div>
            )}
          </div>
          <div className="muted small mono">{local.id}</div>
        </div>
      )}
      <Notice error={act.error ?? list.error} message={act.message} />
      <h2>Outros computadores</h2>
      {others.length === 0 && <div className="empty">Nenhum outro computador sincronizou ainda.</div>}
      <div className="list">
        {others.map((d) => (
          <div key={d.id} className="card stack">
            <div className="row between">
              <div>
                <strong>{d.name}</strong> {d.approved ? <Badge tone="ok">aprovado</Badge> : <Badge>não aprovado</Badge>}
                <div className="muted small">
                  visto pela primeira vez {dateTime(d.first_seen_ms)} · última atividade {relTime(d.last_seen_ms)}
                </div>
              </div>
            </div>
            {d.fingerprint ? (
              <>
                <div className="fingerprint">{d.fingerprint}</div>
                <Toggle
                  checked={d.approved}
                  onChange={(v) => approve(d, v)}
                  label="Permitir aplicação automática dos checkpoints deste computador"
                  hint="Vale só para projetos marcados como confiáveis, e ainda exige avanço direto, sem conflitos e sem editor aberto."
                />
                <div>
                  <button className="link" onClick={() => resetKey(d)}>Redefinir chave (o computador trocou de chave)</button>
                </div>
              </>
            ) : (
              <div className="notice">Este computador ainda não enviou checkpoints assinados; não pode ser aprovado.</div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
