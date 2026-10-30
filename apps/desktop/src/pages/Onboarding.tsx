import { useState } from "react";
import { call } from "../api";
import type { Nav } from "../App";
import { Notice, useAction, useLoad } from "../ui";
import AddProject from "./AddProject";
import GitHubConnect, { useGithub } from "./GitHubConnect";

export default function Onboarding({ onDone }: { nav: Nav; onDone: () => void }) {
  const [step, setStep] = useState(0);
  const device = useLoad(() => call<{ id: string; name: string }>("device.get"), []);
  const gh = useGithub();
  const [name, setName] = useState<string | null>(null);
  const act = useAction();

  const saveName = () =>
    act.run(async () => {
      await call("device.rename", { name: name ?? device.data?.name ?? "" });
      setStep(1);
    });

  return (
    <div className="page narrow">
      <h1>Bem-vindo ao Continuity</h1>
      <p className="lead">
        Continue seu trabalho em outro computador — inclusive alterações não commitadas — sem executar comandos Git. Os checkpoints
        ficam em referências próprias do seu remoto Git; seu staging e sua branch não são alterados.
      </p>
      <ol className="steps">
        {["Este computador", "Conta GitHub", "Primeiro projeto"].map((s, i) => (
          <li key={s} className={i === step ? "active" : i < step ? "done" : ""}>{s}</li>
        ))}
      </ol>
      {step === 0 && (
        <div className="card stack">
          <label className="field">
            <span>Nome deste computador (visível nos outros dispositivos)</span>
            <input value={name ?? device.data?.name ?? ""} onChange={(e) => setName(e.target.value)} />
          </label>
          <p className="muted small">ID persistente: <code>{device.data?.id}</code></p>
          <button className="primary" onClick={saveName} disabled={act.busy}>Continuar</button>
          <Notice error={act.error ?? device.error} />
        </div>
      )}
      {step === 1 && (
        <div className="card stack">
          <GitHubConnect onChange={gh.reload} />
          <button className="primary" onClick={() => setStep(2)}>{gh.data?.connected ? "Continuar" : "Pular (usar credenciais Git existentes)"}</button>
        </div>
      )}
      {step === 2 && <AddProject githubConnected={!!gh.data?.connected} onAdded={onDone} />}
    </div>
  );
}
