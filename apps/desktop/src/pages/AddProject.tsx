import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { call, type Project } from "../api";
import { relTime } from "../format";
import { cloneFolderName, filterRepos, type Repo } from "../repos";
import { Badge, Empty, Notice, Toggle, useAction } from "../ui";

interface Inspection {
  path: string;
  exists: boolean;
  is_repo: boolean;
  workdir: string | null;
  branch: string | null;
  has_commits: boolean;
  remotes: [string, string][];
  already_added: string | null;
}

/** Searchable list of the account's repositories; picking one fills the URL. */
function RepoPicker({ repos, selected, onPick }: { repos: Repo[]; selected: string; onPick: (r: Repo) => void }) {
  const [query, setQuery] = useState("");
  const shown = filterRepos(repos, query);
  return (
    <div className="repo-picker">
      <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder={`Buscar entre ${repos.length} repositório(s)…`} autoFocus />
      <div className="repo-list" role="listbox">
        {shown.map((r) => (
          <button key={r.full_name} role="option" aria-selected={r.clone_url === selected} className={`repo-item ${r.clone_url === selected ? "sel" : ""}`} onClick={() => onPick(r)}>
            <span className="li-main">
              <strong>{r.full_name}</strong>
              {r.description && <span className="muted small">{r.description}</span>}
            </span>
            <span className="li-meta">
              {r.private && <Badge>privado</Badge>}
              {r.updated_at && <span className="muted small">{relTime(Date.parse(r.updated_at))}</span>}
            </span>
          </button>
        ))}
        {!shown.length && <Empty>{repos.length ? "Nenhum repositório corresponde à busca." : "Nenhum repositório nesta conta."}</Empty>}
      </div>
    </div>
  );
}

/** Add a local folder (with or without Git) or clone a remote. */
export default function AddProject({ onAdded, githubConnected }: { onAdded: (p: Project) => void; githubConnected: boolean }) {
  const [mode, setMode] = useState<"local" | "clone">("local");
  const [path, setPath] = useState("");
  const [info, setInfo] = useState<Inspection | null>(null);
  const [initGit, setInitGit] = useState(false);
  const [remoteUrl, setRemoteUrl] = useState("");
  const [cloneUrl, setCloneUrl] = useState("");
  const [cloneDest, setCloneDest] = useState("");
  const [repos, setRepos] = useState<Repo[] | null>(null);
  const [newRepo, setNewRepo] = useState("");
  const act = useAction();
  const reposAct = useAction();

  const pick = async (forClone = false) => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir !== "string") return;
    if (forClone) {
      setCloneDest(dir);
      return;
    }
    setPath(dir);
    const r = await act.run(() => call<Inspection>("path.inspect", { path: dir }));
    if (r) {
      setInfo(r);
      setRemoteUrl(r.remotes.find(([n]) => n === "origin")?.[1] ?? "");
    }
  };

  const loadRepos = () => reposAct.run(async () => setRepos(await call<Repo[]>("github.repos")));

  // Cloning from the connected account is the common case: list it right away.
  useEffect(() => {
    if (mode === "clone" && githubConnected && repos === null && !reposAct.busy && !reposAct.error) loadRepos();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, githubConnected]);

  const createRepo = () =>
    act.run(
      async () => {
        const r = await call<Repo>("github.create_repo", { name: newRepo, private: true });
        setRemoteUrl(r.clone_url);
        return r;
      },
      (r) => `Repositório privado ${r.full_name} criado.`,
    );

  const addLocal = () =>
    act.run(async () => {
      const p = await call<Project>("project.add", { path: info?.workdir ?? path, init_git: initGit, remote_url: remoteUrl || null });
      onAdded(p);
    });

  const doClone = () =>
    act.run(async () => {
      const sep = cloneDest.includes("\\") ? "\\" : "/";
      const p = await call<Project>("project.clone", { url: cloneUrl, dest: `${cloneDest}${sep}${cloneFolderName(cloneUrl)}` });
      onAdded(p);
    });

  const hasOrigin = info?.remotes.some(([n]) => n === "origin");
  return (
    <div className="card">
      <div className="tabs">
        <button className={mode === "local" ? "active" : ""} onClick={() => setMode("local")}>Pasta local</button>
        <button className={mode === "clone" ? "active" : ""} onClick={() => setMode("clone")}>Clonar de um remoto</button>
      </div>
      {mode === "local" ? (
        <>
          <div className="row">
            <input readOnly value={path} placeholder="Selecione a pasta do projeto" />
            <button onClick={() => pick()}>Escolher pasta…</button>
          </div>
          {info && (
            <div className="stack">
              {info.already_added && <div className="notice">Este projeto já está no Elo.</div>}
              {info.is_repo ? (
                <p className="muted">
                  Repositório Git em <code>{info.workdir}</code>, branch <strong>{info.branch ?? "(destacado)"}</strong>
                  {!info.has_commits && " (sem commits)"}.
                </p>
              ) : (
                <Toggle
                  checked={initGit}
                  onChange={setInitGit}
                  label="Inicializar um repositório Git nesta pasta"
                  hint="A pasta não usa Git. O Elo executará `git init` (branch main). Nenhum arquivo é alterado."
                />
              )}
              <label className="field">
                <span>Remoto Git (origin){hasOrigin ? " — já configurado" : ""}</span>
                <input value={remoteUrl} onChange={(e) => setRemoteUrl(e.target.value)} placeholder="https://github.com/usuario/repo.git ou git@…" />
              </label>
              {githubConnected && !hasOrigin && (
                <div className="row wrap">
                  <button onClick={loadRepos} disabled={act.busy || reposAct.busy}>{reposAct.busy ? "Carregando…" : "Escolher dos meus repositórios"}</button>
                  <input value={newRepo} onChange={(e) => setNewRepo(e.target.value)} placeholder="novo-repositorio" />
                  <button onClick={createRepo} disabled={act.busy || !newRepo}>Criar repositório privado</button>
                </div>
              )}
              {repos && !hasOrigin && <RepoPicker repos={repos} selected={remoteUrl} onPick={(r) => setRemoteUrl(r.clone_url)} />}
              <Notice error={reposAct.error} />
              <button className="primary" onClick={addLocal} disabled={act.busy || (!info.is_repo && !initGit) || !!info.already_added}>
                Adicionar projeto
              </button>
            </div>
          )}
        </>
      ) : (
        <div className="stack">
          {githubConnected && (
            <div className="field">
              <span>Seus repositórios no GitHub</span>
              {repos ? (
                <RepoPicker repos={repos} selected={cloneUrl} onPick={(r) => setCloneUrl(r.clone_url)} />
              ) : reposAct.error ? (
                <div className="row">
                  <Notice error={reposAct.error} />
                  <button onClick={loadRepos}>Tentar novamente</button>
                </div>
              ) : (
                <div className="muted small">Carregando repositórios…</div>
              )}
            </div>
          )}
          <label className="field">
            <span>{githubConnected ? "URL do remoto (escolha acima ou cole qualquer URL Git)" : "URL do remoto"}</span>
            <input value={cloneUrl} onChange={(e) => setCloneUrl(e.target.value)} placeholder="https://github.com/usuario/repo.git" />
          </label>
          <div className="row">
            <input readOnly value={cloneDest} placeholder="Pasta onde o clone será criado" />
            <button onClick={() => pick(true)}>Escolher…</button>
          </div>
          {cloneUrl && cloneDest && (
            <div className="muted small">
              Será criado em <code>{`${cloneDest}${cloneDest.includes("\\") ? "\\" : "/"}${cloneFolderName(cloneUrl)}`}</code>
            </div>
          )}
          <button className="primary" onClick={doClone} disabled={act.busy || !cloneUrl || !cloneDest}>
            {act.busy ? "Clonando…" : "Clonar e adicionar"}
          </button>
        </div>
      )}
      <Notice error={act.error} message={act.message} />
    </div>
  );
}
