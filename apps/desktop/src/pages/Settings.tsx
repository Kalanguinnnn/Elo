import { useEffect, useState } from "react";
import { call, type Settings } from "../api";
import { dateTime } from "../format";
import { Notice, Toggle, useAction, useLoad } from "../ui";
import { useMotion } from "../motion";
import { useUpdates } from "../updates";
import { getVersion } from "@tauri-apps/api/app";
import GitHubConnect from "./GitHubConnect";

interface AgentStatus {
  pid: number;
  version: string;
  started_ms: number;
  last_tick_ms: number;
  data_dir: string;
  watched: string[];
  watch_errors: Record<string, string>;
  autostart: boolean;
}

export default function SettingsPage() {
  const s = useLoad(() => call<Settings>("settings.get"), []);
  const agent = useLoad(() => call<AgentStatus>("agent.status"), [], 5000);
  const [draft, setDraft] = useState<Settings | null>(null);
  const act = useAction();
  const motion = useMotion();
  const upd = useUpdates();
  const version = useLoad(() => getVersion(), []);
  useEffect(() => { if (s.data) setDraft(s.data); }, [s.data]);
  if (!draft) return <div className="page"><Notice error={s.error} /></div>;
  const num = (k: keyof Settings, mult = 1) => (e: React.ChangeEvent<HTMLInputElement>) => setDraft({ ...draft, [k]: Number(e.target.value) * mult });
  const ret = (k: keyof Settings["retention"]) => (e: React.ChangeEvent<HTMLInputElement>) => setDraft({ ...draft, retention: { ...draft.retention, [k]: Number(e.target.value) } });

  const save = () => act.run(async () => { setDraft(await call<Settings>("settings.set", { settings: draft })); }, () => "Configurações salvas.");
  const autostart = (v: boolean) => act.run(async () => { await call("agent.autostart", { enable: v }); agent.reload(); s.reload(); });

  return (
    <div className="page">
      <h1>Ajustes</h1>
      <section>
        <h2>GitHub</h2>
        <div className="card"><GitHubConnect /></div>
      </section>
      <section>
        <h2>Sincronização</h2>
        <div className="card stack">
          <label className="field"><span>Intervalo padrão de checkpoints para novos projetos (minutos)</span><input type="number" min={1} value={draft.default_interval_secs / 60} onChange={num("default_interval_secs", 60)} /></label>
          <label className="field"><span>Buscar atualizações a cada (minutos)</span><input type="number" min={1} value={draft.fetch_interval_secs / 60} onChange={num("fetch_interval_secs", 60)} /></label>
          <label className="field"><span>Sem edições por este tempo antes de aplicar automaticamente (segundos)</span><input type="number" min={10} value={draft.quiet_secs} onChange={num("quiet_secs")} /></label>
          <label className="field"><span>Client ID de OAuth App próprio (opcional — vazio usa o app Elo)</span><input value={draft.github_client_id} onChange={(e) => setDraft({ ...draft, github_client_id: e.target.value.trim() })} placeholder="padrão: app Elo" /></label>
        </div>
      </section>
      <section>
        <h2>Retenção progressiva</h2>
        <div className="card stack">
          <div className="grid3">
            <label className="field"><span>Manter tudo das últimas (horas)</span><input type="number" min={0} value={draft.retention.keep_all_hours} onChange={ret("keep_all_hours")} /></label>
            <label className="field"><span>Um por dia durante (dias)</span><input type="number" min={0} value={draft.retention.daily_days} onChange={ret("daily_days")} /></label>
            <label className="field"><span>Um por semana durante (semanas)</span><input type="number" min={0} value={draft.retention.weekly_weeks} onChange={ret("weekly_weeks")} /></label>
            <label className="field"><span>Mínimo por dispositivo</span><input type="number" min={1} value={draft.retention.min_keep} onChange={ret("min_keep")} /></label>
          </div>
          <Toggle checked={draft.prune_remote} onChange={(v) => setDraft({ ...draft, prune_remote: v })} label="Remover do remoto os checkpoints antigos deste computador" hint="Apenas refs refs/continuity deste dispositivo; nunca branches, nunca force push." />
        </div>
      </section>
      <div className="row"><button className="primary" onClick={save} disabled={act.busy}>Salvar</button></div>
      <Notice error={act.error} message={act.message} />
      <section>
        <h2>Aparência</h2>
        <div className="card stack">
          <Toggle
            checked={motion.performance}
            onChange={(v) => motion.set(v ? "off" : "full")}
            label="Modo desempenho"
            hint="Desliga animações e transições da janela. Aplicado na hora, só neste computador."
          />
        </div>
      </section>
      <section>
        <h2>Atualizações</h2>
        <div className="card stack">
          <div className="row between wrap">
            <div>
              <strong>Versão {version.data ?? "—"}</strong>
              <div className="muted small">
                {upd.phase === "checking" && "Procurando…"}
                {upd.phase === "none" && "Você está na versão mais recente."}
                {upd.phase === "available" && `Elo ${upd.version} disponível.`}
                {upd.phase === "downloading" && `Baixando Elo ${upd.version}${upd.progress != null ? ` (${Math.round(upd.progress * 100)}%)` : "…"}`}
                {upd.phase === "installing" && "Instalando; o Elo vai reiniciar."}
                {upd.phase === "idle" && "As atualizações vêm dos releases publicados no GitHub e têm a assinatura conferida antes da instalação."}
              </div>
            </div>
            {upd.phase === "available" ? (
              <button className="primary" onClick={upd.install}>Atualizar agora</button>
            ) : (
              <button onClick={() => upd.check(true)} disabled={upd.phase === "checking" || upd.phase === "downloading" || upd.phase === "installing"}>
                Procurar atualizações
              </button>
            )}
          </div>
          {upd.phase === "available" && upd.notes && <pre className="code release-notes">{upd.notes}</pre>}
          <Notice error={upd.phase === "error" ? upd.error : null} />
          <Toggle checked={upd.auto} onChange={upd.setAuto} label="Procurar atualizações automaticamente" hint="Ao abrir o Elo e a cada 6 horas. A instalação sempre espera você clicar em Atualizar." />
        </div>
      </section>
      <section>
        <h2>Agent em segundo plano</h2>
        <div className="card stack">
          {agent.data ? (
            <div className="small">
              PID {agent.data.pid} · v{agent.data.version} · iniciado {dateTime(agent.data.started_ms)} · {agent.data.watched.length} projeto(s) monitorado(s)
              <div className="muted mono">{agent.data.data_dir}</div>
              {Object.entries(agent.data.watch_errors).map(([k, v]) => <div key={k} className="notice">Monitoramento indisponível para um projeto ({v}); reconciliação periódica em uso.</div>)}
            </div>
          ) : (
            <Notice error={agent.error} />
          )}
          <Toggle checked={!!agent.data?.autostart} onChange={autostart} label="Iniciar o Agent ao entrar no sistema" hint="Mantém a sincronização ativa mesmo com a janela fechada." />
        </div>
      </section>
    </div>
  );
}
