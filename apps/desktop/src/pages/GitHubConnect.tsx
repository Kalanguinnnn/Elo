import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { call } from "../api";
import { Notice, useAction, useLoad } from "../ui";

interface Status {
  connected: boolean;
  login: string | null;
  device_flow_available: boolean;
  askpass_available: boolean;
}

interface DeviceCode {
  device_code: string;
  user_code: string;
  verification_uri: string;
  interval: number;
}

export function useGithub() {
  return useLoad(() => call<Status>("github.status"), []);
}

export default function GitHubConnect({ onChange }: { onChange?: () => void }) {
  const st = useGithub();
  const [token, setToken] = useState("");
  const [code, setCode] = useState<DeviceCode | null>(null);
  const [openFailed, setOpenFailed] = useState(false);
  const act = useAction();

  const saveToken = () =>
    act.run(
      async () => {
        const u = await call<{ login: string }>("github.set_token", { token });
        setToken("");
        await st.reload();
        onChange?.();
        return u;
      },
      (u) => `Conectado como ${u.login}. O token foi guardado no cofre do sistema.`,
    );

  const startDevice = () =>
    act.run(async () => {
      const c = await call<DeviceCode>("github.device_start");
      setCode(c);
      setOpenFailed(false);
      // The code is already on screen; if the browser cannot be opened the
      // user can open the address by hand while we keep waiting.
      await openUrl(c.verification_uri).catch(() => setOpenFailed(true));
      let interval = Math.max(5, c.interval) * 1000;
      for (let i = 0; i < 120; i++) {
        await new Promise((r) => setTimeout(r, interval));
        const r = await call<{ status: string; login?: string; error?: string }>("github.device_poll", { device_code: c.device_code });
        if (r.status === "done") {
          setCode(null);
          await st.reload();
          onChange?.();
          return;
        }
        if (r.status === "slow_down") interval += 5000;
        if (r.status === "failed") throw new Error(`Autorização recusada: ${r.error}`);
      }
      throw new Error("Tempo esgotado aguardando a autorização.");
    });

  const logout = () => act.run(async () => { await call("github.logout"); await st.reload(); onChange?.(); });

  if (!st.data) return <Notice error={st.error} />;
  if (st.data.connected) {
    return (
      <div className="stack">
        <p>
          Conectado como <strong>{st.data.login ?? "(token salvo)"}</strong>. O token está no cofre do sistema operacional e é entregue
          ao Git apenas para github.com.
        </p>
        <div className="row">
          <button onClick={logout} disabled={act.busy}>Desconectar</button>
        </div>
        <Notice error={act.error} message={act.message} />
      </div>
    );
  }
  return (
    <div className="stack">
      {st.data.device_flow_available ? (
        <>
          <button className="primary" onClick={startDevice} disabled={act.busy}>Entrar com GitHub (navegador)</button>
          {code && (
            <div className="notice stack">
              <span>
                Digite o código <strong className="mono">{code.user_code}</strong> em <span className="mono">{code.verification_uri}</span> e
                autorize o Elo. Aguardando a autorização…
              </span>
              {openFailed && <span className="small">Não foi possível abrir o navegador automaticamente; abra o endereço acima manualmente.</span>}
            </div>
          )}
        </>
      ) : (
        <p className="muted small">
          Login pelo navegador requer o client id de um OAuth App (Configurações → GitHub). Enquanto isso, use um token de acesso pessoal.
        </p>
      )}
      <label className="field">
        <span>Token de acesso pessoal (escopo: repositórios)</span>
        <input type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="github_pat_… ou ghp_…" autoComplete="off" />
      </label>
      <div className="row">
        <button onClick={saveToken} disabled={act.busy || !token}>Validar e salvar</button>
        <button
          className="link"
          onClick={() => openUrl("https://github.com/settings/personal-access-tokens/new")}
        >
          Criar token no GitHub
        </button>
      </div>
      <p className="muted small">Sem GitHub? Use qualquer remoto Git: as credenciais já configuradas no seu Git (SSH ou gerenciador de credenciais) continuam valendo.</p>
      <Notice error={act.error} message={act.message} />
    </div>
  );
}
