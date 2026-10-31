import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { call, type ProjectStatus } from "./api";
import { useAction, useLoad, Notice } from "./ui";
import Onboarding from "./pages/Onboarding";
import Projects from "./pages/Projects";
import ProjectDetail from "./pages/ProjectDetail";
import History from "./pages/History";
import Compare from "./pages/Compare";
import Conflicts from "./pages/Conflicts";
import Devices from "./pages/Devices";
import ActivityPage from "./pages/Activity";
import SettingsPage from "./pages/Settings";
import About from "./pages/About";
import { Dot, EloLogo } from "./brand";
import { health } from "./status";
import { useAutoUpdateCheck, useUpdates } from "./updates";
import { IconDevices, IconFolder, IconInfo, IconMerge, IconPulse, IconSliders } from "./icons";

export type Route =
  | { page: "onboarding" }
  | { page: "projects" }
  | { page: "project"; id: string }
  | { page: "history"; id: string }
  | { page: "compare"; id: string; from?: string; to?: string }
  | { page: "conflicts"; id?: string; conflictId?: string }
  | { page: "devices" }
  | { page: "activity" }
  | { page: "settings" }
  | { page: "about" };

export type Nav = (r: Route) => void;

const NAV: { page: Route["page"]; label: string; icon: () => JSX.Element }[] = [
  { page: "projects", label: "Projetos", icon: IconFolder },
  { page: "conflicts", label: "Conflitos", icon: IconMerge },
  { page: "devices", label: "Dispositivos", icon: IconDevices },
  { page: "activity", label: "Atividade", icon: IconPulse },
  { page: "settings", label: "Ajustes", icon: IconSliders },
  { page: "about", label: "Sobre", icon: IconInfo },
];

/** Remount (and replay the entrance animation) only when the user actually moves. */
function routeKey(r: Route): string {
  return [r.page, "id" in r ? r.id : "", "from" in r ? r.from : "", "to" in r ? r.to : "", "conflictId" in r ? r.conflictId : ""].join("|");
}

export default function App() {
  const [route, setRoute] = useState<Route>({ page: "projects" });
  const projects = useLoad(() => call<ProjectStatus[]>("projects.list"), [], 8000);
  const quit = useAction();
  const upd = useUpdates();
  useAutoUpdateCheck();
  const nav: Nav = (r) => setRoute(r);

  const conflicts = (projects.data ?? []).reduce((n, p) => n + p.open_conflicts, 0);
  const h = health(projects.data, projects.error);
  const firstRun = projects.data !== null && projects.data.length === 0 && route.page === "projects";

  const syncAndQuit = () =>
    quit.run(
      () => invoke<{ name: string; error: string | null; held: number }[]>("sync_and_quit"),
      (rep) => {
        const bad = rep.filter((r) => r.error || r.held > 0);
        return bad.length
          ? `Não encerrado: ${bad.map((b) => `${b.name}: ${b.error ?? `${b.held} checkpoint(s) retido(s) por segredos`}`).join("; ")}`
          : null;
      },
    );

  let body: React.ReactNode;
  if (projects.error && projects.data === null) {
    body = (
      <div className="page">
        <h1>Agent indisponível</h1>
        <Notice error={projects.error} />
        <button onClick={projects.reload}>Tentar novamente</button>
      </div>
    );
  } else if (firstRun || route.page === "onboarding") {
    body = <Onboarding nav={nav} onDone={() => { projects.reload(); setRoute({ page: "projects" }); }} />;
  } else {
    switch (route.page) {
      case "projects":
        body = <Projects nav={nav} list={projects.data ?? []} reload={projects.reload} />;
        break;
      case "project":
        body = <ProjectDetail nav={nav} id={route.id} onChanged={projects.reload} />;
        break;
      case "history":
        body = <History nav={nav} id={route.id} />;
        break;
      case "compare":
        body = <Compare nav={nav} id={route.id} from={route.from} to={route.to} />;
        break;
      case "conflicts":
        body = <Conflicts nav={nav} projectId={route.id} conflictId={route.conflictId} onChanged={projects.reload} />;
        break;
      case "devices":
        body = <Devices />;
        break;
      case "activity":
        body = <ActivityPage projects={projects.data ?? []} />;
        break;
      case "settings":
        body = <SettingsPage />;
        break;
      case "about":
        body = <About />;
        break;
    }
  }

  return (
    <div className="shell">
      <aside className="sidebar">
        <EloLogo size={21} />
        <nav>
          {NAV.map((n, i) => (
            <div key={n.page}>
              {i === 2 && <div className="nav-label">Geral</div>}
              <button
                className={`nav ${route.page === n.page || (n.page === "projects" && ["project", "history", "compare"].includes(route.page)) ? "active" : ""}`}
                onClick={() => setRoute({ page: n.page } as Route)}
              >
                <n.icon />
                {n.label}
                {n.page === "conflicts" && conflicts > 0 && <span className="count">{conflicts}</span>}
              </button>
            </div>
          ))}
        </nav>
        <div className="sidebar-foot">
          {(upd.phase === "available" || upd.phase === "downloading" || upd.phase === "installing") && (
            <div className="update-card">
              <strong>Elo {upd.version} disponível</strong>
              {upd.phase === "available" && <button className="primary block" onClick={upd.install}>Atualizar agora</button>}
              {upd.phase === "downloading" && (
                <>
                  <span className="muted small">Baixando{upd.progress != null ? ` ${Math.round(upd.progress * 100)}%` : "…"}</span>
                  <div className="progress"><div style={{ width: `${Math.round((upd.progress ?? 0) * 100)}%` }} /></div>
                </>
              )}
              {upd.phase === "installing" && <span className="muted small">Instalando; o Elo vai reiniciar.</span>}
            </div>
          )}
          <div className="health">
            <Dot tone={h.tone} />
            {h.text}
          </div>
          <button className="primary block" disabled={quit.busy} onClick={syncAndQuit} title="Cria checkpoints, envia tudo e encerra o Agent e o aplicativo">
            {quit.busy ? "Sincronizando…" : "Sincronizar e encerrar"}
          </button>
          <Notice error={quit.error} message={quit.message} />
        </div>
      </aside>
      <main className="content">
        <div className="route" key={routeKey(route)}>{body}</div>
      </main>
    </div>
  );
}
