import { useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { call, type ApplyOutcome, type ApplyPreview, type Checkpoint, type Finding, type ProjectStatus } from "../api";
import type { Nav } from "../App";
import { dateTime, relTime, shortBranch } from "../format";
import { Badge, Empty, Notice, Toggle, useAction, useLoad } from "../ui";
import { statusBadges } from "./Projects";
import { IconChevronLeft } from "../icons";

interface Incoming {
  checkpoint: Checkpoint;
  same_branch: boolean;
}

interface Operation {
  id: string;
  kind: string;
  state: string;
  safety_checkpoint: string | null;
  details: string;
  started_ms: number;
}

export function outcomeText(o: ApplyOutcome): string {
  switch (o.outcome) {
    case "applied": {
      const r = o.result;
      const integ = r.integrity_failures.length ? ` ATENÇÃO: ${r.integrity_failures.length} arquivo(s) falharam na verificação de integridade.` : " Integridade verificada.";
      return `Aplicado: ${r.written.length} escrito(s), ${r.deleted.length} removido(s)${r.head_updated ? ", HEAD atualizado" : ""}${r.index_updated ? ", staging atualizado" : ""}.${integ} Estado anterior salvo em ${o.safety_checkpoint}.`;
    }
    case "nothing_to_do":
      return "Nada a aplicar: o estado já é equivalente.";
    case "conflicts":
      return `${o.count} conflito(s). Nenhum arquivo foi alterado; resolva-os lado a lado.`;
    case "blocked":
      return `Bloqueado: ${o.plan.blockers.join("; ")}`;
    case "needs_manual":
      return `Ação manual necessária: ${o.reason}`;
  }
}

function IncomingCard({ projectId, inc, onDone, nav }: { projectId: string; inc: Incoming; onDone: () => void; nav: Nav }) {
  const [preview, setPreview] = useState<ApplyPreview | null>(null);
  const [allowBranch, setAllowBranch] = useState(false);
  const [filesOnly, setFilesOnly] = useState(false);
  const act = useAction();
  const options = { allow_branch_switch: allowBranch, files_only: filesOnly };
  const c = inc.checkpoint;

  const doPreview = () => act.run(async () => setPreview(await call<ApplyPreview>("apply.plan", { project_id: projectId, checkpoint_id: c.id, options })));
  const doApply = () =>
    act.run(
      async () => {
        const o = await call<ApplyOutcome>("apply.run", { project_id: projectId, checkpoint_id: c.id, options });
        if (o.outcome === "conflicts") nav({ page: "conflicts", id: projectId, conflictId: o.conflict_id });
        else if (o.outcome === "applied" || o.outcome === "nothing_to_do") onDone();
        return o;
      },
      outcomeText,
    );

  const p = preview?.plan;
  return (
    <div className="card">
      <div className="row between">
        <div>
          <strong>{c.device_name}</strong> · {relTime(c.created_ms)} · branch {shortBranch(c.branch)} {!inc.same_branch && <Badge tone="warn">outra branch</Badge>}{" "}
          {c.verified ? <Badge tone="ok">assinatura verificada</Badge> : <Badge tone="err">origem não verificada</Badge>}
          <div className="muted small">{c.summary} · checkpoint {c.id}</div>
        </div>
        <div className="row">
          <button onClick={doPreview} disabled={act.busy}>Pré-visualizar</button>
          <button className="primary" onClick={doApply} disabled={act.busy || !preview}>Aplicar</button>
        </div>
      </div>
      {p && (
        <div className="stack top">
          <div className="muted small">
            {p.changes.length} arquivo(s) a atualizar · {p.conflicts.length} conflito(s) · {p.skipped.length} ignorado(s) ·{" "}
            {p.local_changes ? "há alterações locais (mesclagem)" : "sem alterações locais (avanço direto)"} · IDE: {preview!.ide.reason}
          </div>
          {p.changes.length > 0 && (
            <details>
              <summary>Arquivos</summary>
              <ul className="mono small">
                {p.changes.map((ch) => (
                  <li key={ch.path}>{ch.action.kind === "delete" ? "− " : "＋ "}{ch.path}</li>
                ))}
              </ul>
            </details>
          )}
          {!preview!.origin_verified && (
            <div className="notice error">
              A assinatura deste checkpoint não confere com o computador "{c.device_name}". Ele pode ter sido forjado por alguém com acesso
              de escrita ao remoto — só aplique se tiver certeza da origem.
            </div>
          )}
          {preview!.origin_verified && !preview!.device_approved && (
            <div className="notice">"{c.device_name}" ainda não está aprovado em Dispositivos, por isso atualizações dele nunca são aplicadas automaticamente.</div>
          )}
          {p.skipped.map((s) => <div key={s.path} className="notice">Não será aplicado: {s.path} — {s.reason}</div>)}
          {p.blockers.map((b) => <div key={b} className="notice error">{b}</div>)}
          {p.notes.map((n) => <div key={n} className="notice">{n}</div>)}
          {p.needs_branch_confirmation && (
            <Toggle
              checked={allowBranch}
              onChange={(v) => { setAllowBranch(v); setPreview(null); }}
              label={`Trocar para a branch ${shortBranch(p.theirs_branch)}`}
              hint={`Este computador está em ${shortBranch(p.ours_branch)}. A troca só ocorre com esta confirmação explícita.`}
            />
          )}
          {p.blockers.length > 0 && (
            <Toggle
              checked={filesOnly}
              onChange={(v) => { setFilesOnly(v); setPreview(null); }}
              label="Aplicar apenas os arquivos"
              hint="Não altera HEAD, branch nem staging deste computador."
            />
          )}
          {!preview?.plan.blockers.length && preview?.auto_eligible && <Badge tone="ok">aplicação segura (avanço direto)</Badge>}
        </div>
      )}
      <Notice error={act.error} message={act.message} />
    </div>
  );
}

function Secrets({ projectId, onDone }: { projectId: string; onDone: () => void }) {
  const f = useLoad(() => call<Finding[]>("secrets.list", { project_id: projectId }), [projectId]);
  const [sel, setSel] = useState<Record<string, boolean>>({});
  const act = useAction();
  if (!f.data?.length) return null;
  const approve = () =>
    act.run(
      async () => {
        const fps = Object.keys(sel).filter((k) => sel[k]);
        const n = await call<number>("secrets.approve", { project_id: projectId, fingerprints: fps });
        f.reload();
        onDone();
        return n;
      },
      (n) => `${n} alerta(s) confirmado(s); o envio será retomado.`,
    );
  return (
    <section>
      <h2>Possíveis segredos</h2>
      <div className="card stack">
        <p className="small">
          Checkpoints com credenciais <strong>bloqueantes</strong> nunca são enviados: remova o conteúdo ou adicione o arquivo ao
          .gitignore e um novo checkpoint seguirá normalmente. Alertas podem ser confirmados individualmente.
        </p>
        {f.data.map((x) => (
          <label key={x.checkpoint_id + x.fingerprint} className="finding">
            <input
              type="checkbox"
              disabled={x.severity === "blocking" || x.allowed}
              checked={x.allowed || !!sel[x.fingerprint]}
              onChange={(e) => setSel({ ...sel, [x.fingerprint]: e.target.checked })}
            />
            <Badge tone={x.severity === "blocking" ? "err" : "warn"}>{x.severity === "blocking" ? "bloqueante" : "alerta"}</Badge>
            <span className="mono small">{x.path}{x.line ? `:${x.line}` : ""}</span>
            <span className="small">{x.rule}</span>
            <span className="muted small mono">{x.excerpt}</span>
          </label>
        ))}
        <button onClick={approve} disabled={act.busy || !Object.values(sel).some(Boolean)}>Confirmar envio dos alertas marcados</button>
        <Notice error={act.error} message={act.message} />
      </div>
    </section>
  );
}

export default function ProjectDetail({ nav, id, onChanged }: { nav: Nav; id: string; onChanged: () => void }) {
  const st = useLoad(() => call<ProjectStatus>("project.status", { project_id: id }), [id], 6000);
  const incoming = useLoad(() => call<Incoming[]>("incoming.list", { project_id: id }), [id], 10000);
  const ops = useLoad(() => call<Operation[]>("operations.list", { project_id: id, limit: 20 }), [id]);
  const [remote, setRemote] = useState<string | null>(null);
  const act = useAction();

  const refresh = () => { st.reload(); incoming.reload(); ops.reload(); onChanged(); };
  const patch = (p: Record<string, unknown>) => act.run(async () => { await call("project.update", { project_id: id, patch: p }); refresh(); });

  if (!st.data) return <div className="page"><Notice error={st.error} /></div>;
  const s = st.data;
  const pr = s.project;
  const interrupted = (ops.data ?? []).filter((o) => o.state === "interrupted");

  const setTrust = async (v: boolean) => {
    if (v) {
      const ok = await ask(
        "Em um projeto confiável, atualizações de outros computadores são aplicadas automaticamente — somente quando forem um avanço direto, sem alterações locais, sem conflitos, na mesma branch, sem editor/IDE em execução e sem edições recentes. Em qualquer outra situação será pedida ação manual. Continuar?",
        { title: "Confiar neste projeto", kind: "warning" },
      );
      if (!ok) return;
    }
    patch({ trusted: v });
  };
  const setAutocommit = async (v: boolean) => {
    if (v) {
      const ok = await ask(
        "O modo de commits automáticos cria commits reais (chore(continuity): …) na branch atual e os envia sem force push. Ele é adiado sempre que houver staging parcial. Continuar?",
        { title: "Commits automáticos", kind: "warning" },
      );
      if (!ok) return;
    }
    patch({ autocommit: v });
  };
  const remove = async () => {
    const ok = await ask(`Parar de sincronizar "${pr.name}"? Arquivos, refs e o remoto não são alterados.`, { title: "Remover projeto", kind: "warning" });
    if (!ok) return;
    act.run(async () => { await call("project.remove", { project_id: id }); onChanged(); nav({ page: "projects" }); });
  };

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <button className="link back" onClick={() => nav({ page: "projects" })}><IconChevronLeft />Projetos</button>
          <h1>{pr.name}</h1>
          <div className="muted mono small">{pr.path}</div>
        </div>
        <div className="row">
          <button onClick={() => act.run(() => call<Checkpoint | null>("checkpoint.create", { project_id: id }), (c) => (c ? `Checkpoint ${c.id} criado.` : "Nada mudou desde o último checkpoint."))} disabled={act.busy}>
            Criar checkpoint
          </button>
          <button className="primary" onClick={() => act.run(async () => { const r = await call<{ imported: number; incoming: number }>("sync.project", { project_id: id }); refresh(); return r; }, (r) => `Sincronizado: ${r.imported} recebido(s), ${r.incoming} atualização(ões) disponível(is).`)} disabled={act.busy}>
            Sincronizar agora
          </button>
        </div>
      </div>
      <Notice error={act.error} message={act.message} />
      <div className="badges">{statusBadges(s)}</div>

      <div className="grid3">
        <div className="stat"><span>Branch</span><strong>{s.branch ?? "(destacado)"}</strong></div>
        <div className="stat"><span>Alterações locais</span><strong>{s.uncommitted_changes ?? "—"}</strong></div>
        <div className="stat"><span>Aguardando envio</span><strong>{s.pending_push}</strong></div>
        <div className="stat"><span>Último checkpoint</span><strong>{relTime(pr.last_checkpoint_ms)}</strong></div>
        <div className="stat"><span>Último envio</span><strong>{relTime(pr.last_push_ms)}</strong></div>
        <div className="stat"><span>Última busca</span><strong>{relTime(pr.last_fetch_ms)}</strong></div>
      </div>
      {s.git_busy && <div className="notice">Operação Git em andamento ({s.git_busy}); checkpoints aguardam a conclusão.</div>}
      {s.queue_error && <div className={`notice ${s.queue_failure_kind === "offline" ? "" : "error"}`}>Fila: {s.queue_error}</div>}
      {s.open_conflicts > 0 && (
        <div className="notice error row between">
          {s.open_conflicts} conflito(s) aguardando decisão.
          <button onClick={() => nav({ page: "conflicts", id })}>Resolver</button>
        </div>
      )}
      {interrupted.map((o) => (
        <div key={o.id} className="notice row between">
          <span>Operação “{o.kind}” interrompida em {dateTime(o.started_ms)}. Estado anterior: checkpoint {o.safety_checkpoint}.</span>
          <span className="row">
            <button onClick={() => nav({ page: "history", id })}>Ver histórico</button>
            <button onClick={() => act.run(async () => { await call("operation.ack", { op_id: o.id }); ops.reload(); st.reload(); })}>Dispensar</button>
          </span>
        </div>
      ))}

      <section>
        <div className="row between">
          <h2>Atualizações de outros computadores</h2>
          <div className="row">
            <button onClick={() => nav({ page: "history", id })}>Histórico</button>
            <button onClick={() => nav({ page: "compare", id })}>Comparar versões</button>
          </div>
        </div>
        {incoming.error && <Notice error={incoming.error} />}
        {(incoming.data ?? []).length === 0 ? (
          <Empty>Nenhuma atualização pendente.</Empty>
        ) : (
          incoming.data!.map((inc) => <IncomingCard key={inc.checkpoint.id} projectId={id} inc={inc} nav={nav} onDone={refresh} />)
        )}
      </section>

      <Secrets projectId={id} onDone={refresh} />

      <section>
        <h2>Configuração</h2>
        <div className="card stack">
          <label className="field">
            <span>Remoto ({pr.remote})</span>
            <div className="row">
              <input value={remote ?? s.remote_url ?? ""} onChange={(e) => setRemote(e.target.value)} placeholder="URL do remoto Git" />
              <button disabled={act.busy || remote === null} onClick={() => act.run(async () => { await call("project.set_remote", { project_id: id, url: remote }); setRemote(null); st.reload(); }, () => "Remoto atualizado.")}>Salvar</button>
            </div>
          </label>
          <label className="field">
            <span>Intervalo de checkpoints (minutos)</span>
            <input type="number" min={1} max={1440} key={pr.interval_secs} defaultValue={Math.round(pr.interval_secs / 60)} onBlur={(e) => { const m = Math.max(1, Number(e.target.value) || 10); if (m * 60 !== pr.interval_secs) patch({ interval_secs: m * 60 }); }} />
          </label>
          <Toggle checked={pr.auto_push} onChange={(v) => patch({ auto_push: v })} label="Enviar checkpoints automaticamente" hint="Quando offline, os envios ficam na fila e são repetidos com espera crescente." />
          <Toggle checked={pr.trusted} onChange={setTrust} label="Projeto confiável (aplicação automática segura)" hint="Desativado por padrão. Mesmo ativado, qualquer dúvida exige ação manual." />
          <Toggle checked={pr.autocommit} onChange={setAutocommit} label="Commits Git automáticos (modo convencional)" hint="Opcional. Cria commits reais na branch; checkpoints continuam sendo criados." />
          <Toggle checked={pr.paused} onChange={(v) => patch({ paused: v })} label="Pausar sincronização" />
          <div className="row">
            <button onClick={() => act.run(() => call<{ pruned_local: number; queued_remote: number; kept: number }>("retention.run", { project_id: id }), (r) => `Retenção: ${r.pruned_local} removido(s), ${r.queued_remote} agendado(s) no remoto, ${r.kept} mantido(s).`)}>
              Aplicar retenção agora
            </button>
            <button className="danger" onClick={remove}>Remover do Continuity</button>
          </div>
        </div>
      </section>
    </div>
  );
}
