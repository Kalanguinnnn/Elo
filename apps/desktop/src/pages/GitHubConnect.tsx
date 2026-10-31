import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { call } from "../api";
import { Notice, useAction, useLoad } from "../ui";

interface Status {
  connected: boolean;
  /** Live check: false = GitHub rejects the stored token; null = unreachable. */
  token_valid: boolean | null;
  login: string | null;
  device_flow_available: boolean;
  askpass_available: boolean;
}

interface DeviceCode {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export function useGithub() {
  return useLoad(() => call<Status>("github.status"), []);
}

export function GitHubLogo({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}

async function copy(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** The one-time code, large and boxed like on github.com/login/device. */
function CodeBoxes({ code }: { code: string }) {
  return (
    <div className="code-boxes" aria-label={`Código ${code}`}>
      {code.split("").map((ch, i) =>
        ch === "-" ? (
          <span key={i} className="code-sep">–</span>
        ) : (
          <span key={i} className="code-char">{ch}</span>
        ),
      )}
    </div>
  );
}

export function DeviceLogin({ code, onCancel }: { code: DeviceCode; onCancel: () => void }) {
  const [copied, setCopied] = useState<boolean | null>(null);
  const [left, setLeft] = useState(code.expires_in);
  useEffect(() => {
    copy(code.user_code).then(setCopied);
    const t = setInterval(() => setLeft((s) => Math.max(0, s - 1)), 1000);
    return () => clearInterval(t);
  }, [code]);
  const mm = String(Math.floor(left / 60)).padStart(2, "0");
  const ss = String(left % 60).padStart(2, "0");
  return (
    <div className="device-login">
      <p className="device-title">Digite este código no GitHub</p>
      <CodeBoxes code={code.user_code} />
      <div className="row center">
        <button onClick={async () => setCopied(await copy(code.user_code))}>{copied ? "Código copiado ✓" : "Copiar código"}</button>
        <button onClick={() => openUrl(code.verification_uri).catch(() => undefined)}>Abrir o GitHub de novo</button>
      </div>
      <ol className="howto">
        <li><strong>Copie o código acima</strong>{copied ? " — já está na sua área de transferência." : "."}</li>
        <li>Na página do GitHub que abrimos (<span className="mono">{code.verification_uri.replace("https://", "")}</span>), <strong>cole o código</strong> e clique em <em>Continue</em>.</li>
        <li>Clique em <strong>Authorize Elo</strong>. Pronto — o Elo detecta sozinho.</li>
      </ol>
      <div className="waiting">
        <span className="pulse" />
        Aguardando sua autorização no GitHub… <span className="muted">o código expira em {mm}:{ss}</span>
      </div>
      <button className="link" onClick={onCancel}>Cancelar</button>
    </div>
  );
}

export default function GitHubConnect({ onChange }: { onChange?: () => void }) {
  const st = useGithub();
  const [token, setToken] = useState("");
  const [code, setCode] = useState<DeviceCode | null>(null);
  const cancelled = useRef(false);
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
      cancelled.current = false;
      const c = await call<DeviceCode>("github.device_start");
      setCode(c);
      // The code is already on screen; if the browser cannot be opened the
      // user can still use "Abrir o GitHub de novo" while we keep waiting.
      await openUrl(c.verification_uri).catch(() => undefined);
      let interval = Math.max(5, c.interval) * 1000;
      const deadline = Date.now() + c.expires_in * 1000;
      while (Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, interval));
        if (cancelled.current) return;
        const r = await call<{ status: string; login?: string; error?: string }>("github.device_poll", { device_code: c.device_code });
        if (r.status === "done") {
          setCode(null);
          await st.reload();
          onChange?.();
          return;
        }
        if (r.status === "slow_down") interval += 5000;
        if (r.status === "failed") {
          setCode(null);
          throw new Error(r.error === "access_denied" ? "A autorização foi recusada no GitHub." : `O GitHub recusou o login (${r.error}).`);
        }
      }
      setCode(null);
      throw new Error("O código expirou. Clique em “Entrar com GitHub” para gerar outro.");
    });

  const cancel = () => {
    cancelled.current = true;
    setCode(null);
  };

  const logout = () => act.run(async () => { await call("github.logout"); await st.reload(); onChange?.(); });

  if (!st.data) return <Notice error={st.error} />;
  if (st.data.connected) {
    return (
      <div className="gh-connected">
        <span className="gh-avatar"><GitHubLogo size={20} /></span>
        <div className="grow">
          <strong>Conectado como {st.data.login ?? "(conta GitHub)"}</strong>
          <div className="muted small">O token está no cofre do sistema e só é entregue ao Git para github.com.</div>
          {st.data.token_valid === false && (
            <div className="notice error">O GitHub recusou o token salvo (revogado ou expirado). Desconecte e entre de novo para voltar a sincronizar.</div>
          )}
        </div>
        <button onClick={logout} disabled={act.busy}>Desconectar</button>
        <Notice error={act.error} message={act.message} />
      </div>
    );
  }
  return (
    <div className="stack">
      {code ? (
        <DeviceLogin code={code} onCancel={cancel} />
      ) : (
        <div className="gh-hero">
          <span className="gh-avatar big"><GitHubLogo size={30} /></span>
          <p className="device-title">Conecte sua conta do GitHub</p>
          <p className="muted small center-text">
            Vamos abrir o GitHub no navegador e mostrar um código aqui. Você só precisa colar o código lá e autorizar. Nenhuma senha é
            digitada no Elo.
          </p>
          <button className="primary gh-button" onClick={startDevice} disabled={act.busy || !st.data.device_flow_available}>
            <GitHubLogo /> Entrar com GitHub
          </button>
          {!st.data.device_flow_available && <p className="muted small">Login pelo navegador indisponível nesta build; use um token em “Outras opções”.</p>}
        </div>
      )}
      <Notice error={act.error} message={act.message} />
      <details className="other-options">
        <summary>Outras opções</summary>
        <div className="stack top">
          <label className="field">
            <span>Token de acesso pessoal (permissões: repositórios e workflows)</span>
            <input type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="github_pat_… ou ghp_…" autoComplete="off" />
          </label>
          <div className="row">
            <button onClick={saveToken} disabled={act.busy || !token}>Validar e salvar</button>
            <button className="link" onClick={() => openUrl("https://github.com/settings/personal-access-tokens/new")}>Criar token no GitHub</button>
          </div>
          <p className="muted small">
            Sem GitHub? Use qualquer remoto Git: as credenciais já configuradas no seu Git (SSH ou gerenciador de credenciais) continuam
            valendo.
          </p>
        </div>
      </details>
    </div>
  );
}
