import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { call, type Project } from "../api";
import { Notice, Toggle, useAction } from "../ui";

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

interface Repo {
  full_name: string;
  private: boolean;
  clone_url: string;
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

  const loadRepos = () => act.run(async () => setRepos(await call<Repo[]>("github.repos")));

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
      const name = cloneUrl.replace(/\.git$/, "").split(/[/:]/).pop() || "projeto";
      const p = await call<Project>("project.clone", { url: cloneUrl, dest: `${cloneDest}${sep}${name}` });
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
              {info.already_added && <div className="notice">Este projeto já está no Continuity.</div>}
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
                  hint="A pasta não usa Git. O Continuity executará `git init` (branch main). Nenhum arquivo é alterado."
                />
              )}
              <label className="field">
                <span>Remoto Git (origin){hasOrigin ? " — já configurado" : ""}</span>
                <input value={remoteUrl} onChange={(e) => setRemoteUrl(e.target.value)} placeholder="https://github.com/usuario/repo.git ou git@…" />
              </label>
              {githubConnected && !hasOrigin && (
                <div className="row wrap">
                  <button onClick={loadRepos} disabled={act.busy}>Escolher dos meus repositórios</button>
                  <input value={newRepo} onChange={(e) => setNewRepo(e.target.value)} placeholder="novo-repositorio" />
                  <button onClick={createRepo} disabled={act.busy || !newRepo}>Criar repositório privado</button>
                </div>
              )}
              {repos && (
                <select onChange={(e) => setRemoteUrl(e.target.value)} value={remoteUrl}>
                  <option value="">— selecione —</option>
                  {repos.map((r) => (
                    <option key={r.full_name} value={r.clone_url}>
                      {r.full_name} {r.private ? "(privado)" : ""}
                    </option>
                  ))}
                </select>
              )}
              <button className="primary" onClick={addLocal} disabled={act.busy || (!info.is_repo && !initGit) || !!info.already_added}>
                Adicionar projeto
              </button>
            </div>
          )}
        </>
      ) : (
        <div className="stack">
          <label className="field">
            <span>URL do remoto</span>
            <input value={cloneUrl} onChange={(e) => setCloneUrl(e.target.value)} placeholder="https://github.com/usuario/repo.git" />
          </label>
          <div className="row">
            <input readOnly value={cloneDest} placeholder="Pasta onde o clone será criado" />
            <button onClick={() => pick(true)}>Escolher…</button>
          </div>
          <button className="primary" onClick={doClone} disabled={act.busy || !cloneUrl || !cloneDest}>
            {act.busy ? "Clonando…" : "Clonar e adicionar"}
          </button>
        </div>
      )}
      <Notice error={act.error} message={act.message} />
    </div>
  );
}
