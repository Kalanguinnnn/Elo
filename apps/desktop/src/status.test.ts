import { describe, expect, it } from "vitest";
import type { ProjectStatus } from "./api";
import { health } from "./status";
import { statusBadges } from "./pages/Projects";

function st(over: Partial<ProjectStatus> = {}): ProjectStatus {
  return {
    project: {
      id: "p", name: "demo", path: "/x", remote: "origin", trusted: false, paused: false, interval_secs: 600,
      auto_push: true, autocommit: false, lineage_head: null, last_checkpoint_ms: null, last_fetch_ms: null,
      last_push_ms: null, last_error: null,
    },
    available: true, branch: "main", head: null, remote_url: null, uncommitted_changes: 0, pending_push: 0,
    needs_review: 0, quarantined: 0, incoming: 0, open_conflicts: 0, interrupted_ops: 0, queue_error: null,
    queue_failure_kind: null, git_busy: null, unsafe_config: null,
    ...over,
  };
}
const keys = (s: ProjectStatus) => statusBadges(s).map((b) => (b as { key: string }).key);

describe("sidebar health", () => {
  it("reports loading, errors and empty state", () => {
    expect(health(null, "x").tone).toBe("err");
    expect(health(null, null).text).toBe("Conectando…");
    expect(health([], null).text).toBe("Nenhum projeto");
  });
  it("orders problems by urgency", () => {
    expect(health([st()], null)).toEqual({ tone: "ok", text: "Tudo em dia" });
    expect(health([st({ pending_push: 2 })], null).tone).toBe("warn");
    expect(health([st({ incoming: 1 }), st({ pending_push: 1 })], null).tone).toBe("info");
    expect(health([st({ queue_failure_kind: "offline", incoming: 1 })], null).text).toBe("Offline — na fila");
    for (const bad of [{ open_conflicts: 1 }, { quarantined: 1 }, { unsafe_config: "filter.x.clean" }]) {
      expect(health([st(), st(bad)], null).text).toBe("Requer sua atenção");
    }
  });
});

describe("project badges", () => {
  it("shows 'Tudo em dia' only when nothing else applies", () => {
    expect(keys(st())).toEqual(["ok"]);
    expect(keys(st({ project: { ...st().project, trusted: true } }))).toEqual(["t"]);
  });
  it("never hides an unsafe Git config behind other states", () => {
    expect(keys(st({ unsafe_config: "x", queue_failure_kind: "offline" }))).toEqual(["u", "o"]);
    expect(keys(st({ unsafe_config: "x", queue_error: "boom" }))).toEqual(["u", "e"]);
  });
  it("flags unavailable folders, conflicts and blocked credentials", () => {
    expect(keys(st({ available: false }))).toEqual(["na"]);
    expect(keys(st({ open_conflicts: 2, quarantined: 1, incoming: 3 }))).toEqual(["c", "i", "q"]);
  });
});
