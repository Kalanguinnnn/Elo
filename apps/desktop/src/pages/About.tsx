import { call } from "../api";
import { EloLogo } from "../brand";
import { useLoad } from "../ui";

export default function About() {
  const ping = useLoad(() => call<{ version: string }>("ping"), []);
  return (
    <div className="page narrow">
      <EloLogo size={34} />
      <div className="hero">
        Continue
        <br />
        de onde parou<em>.</em>
      </div>
      <p className="muted">Versão {ping.data?.version ?? "—"} · beta</p>
      <div className="group">
        <div className="row-item">Checkpoints são commits Git em <code>refs/continuity/checkpoints/…</code> no <em>seu</em> remoto.</div>
        <div className="row-item">Staging, HEAD e branch não são alterados ao criar checkpoints.</div>
        <div className="row-item">Nunca usa force push; checkpoints são imutáveis.</div>
        <div className="row-item">Tokens ficam no cofre do sistema operacional.</div>
        <div className="row-item">Possíveis segredos são verificados antes de cada envio.</div>
        <div className="row-item">Nenhum conteúdo de projeto é enviado a servidores do Elo — eles não existem.</div>
      </div>
      <p className="muted small" style={{ marginTop: 18 }}>Gratuito e de código aberto. Licença: MIT ou Apache-2.0 (a definir).</p>
    </div>
  );
}
