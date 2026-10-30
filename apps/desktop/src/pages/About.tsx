import { openUrl } from "@tauri-apps/plugin-opener";
import { call } from "../api";
import { useLoad } from "../ui";

export default function About() {
  const ping = useLoad(() => call<{ version: string }>("ping"), []);
  return (
    <div className="page narrow">
      <h1>Sobre</h1>
      <p><strong>Continuity</strong> {ping.data ? `v${ping.data.version}` : ""} — nome provisório.</p>
      <p>
        Aplicativo gratuito e de código aberto para continuar o desenvolvimento entre computadores. Checkpoints são commits Git em
        <code> refs/continuity/checkpoints/…</code> no <em>seu</em> remoto; nenhum conteúdo de projeto é enviado a servidores do
        Continuity — não existem servidores do Continuity.
      </p>
      <ul className="small">
        <li>Staging, HEAD e branch não são alterados ao criar checkpoints.</li>
        <li>Nunca usa force push; refs de checkpoint são imutáveis.</li>
        <li>Tokens ficam no cofre do sistema operacional.</li>
        <li>Possíveis segredos são verificados antes de cada envio.</li>
      </ul>
      <p className="muted small">Licença: MIT ou Apache-2.0 (a definir).</p>
      <button className="link" onClick={() => openUrl("https://git-scm.com/docs/git-update-ref")}>Como funcionam refs Git</button>
    </div>
  );
}
