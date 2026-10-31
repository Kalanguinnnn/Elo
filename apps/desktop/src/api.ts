// Typed access to the Agent. Every call goes through the `agent_call` Tauri
// command, which forwards it over local IPC; nothing here is mocked.
import { invoke } from "@tauri-apps/api/core";

export async function call<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
  if (!("__TAURI_INTERNALS__" in window)) {
    throw new Error("Esta interface precisa ser aberta pelo aplicativo Elo (o Agent não é acessível pelo navegador).");
  }
  return invoke<T>("agent_call", { method, params });
}

export interface Project {
  id: string;
  name: string;
  path: string;
  remote: string;
  trusted: boolean;
  paused: boolean;
  interval_secs: number;
  auto_push: boolean;
  autocommit: boolean;
  lineage_head: string | null;
  last_checkpoint_ms: number | null;
  last_fetch_ms: number | null;
  last_push_ms: number | null;
  last_error: string | null;
}

export interface ProjectStatus {
  project: Project;
  available: boolean;
  branch: string | null;
  head: string | null;
  remote_url: string | null;
  uncommitted_changes: number | null;
  pending_push: number;
  needs_review: number;
  quarantined: number;
  incoming: number;
  open_conflicts: number;
  interrupted_ops: number;
  queue_error: string | null;
  queue_failure_kind: string | null;
  git_busy: string | null;
  unsafe_config: string | null;
}

export interface Checkpoint {
  id: string;
  device_id: string;
  device_name: string;
  commit_oid: string;
  tree_oid: string;
  head_oid: string | null;
  branch: string | null;
  created_ms: number;
  trigger: string;
  parents: string[];
  summary: string;
  files_changed: number | null;
  origin: string;
  push_state: string;
  applied_ms: number | null;
  verified: boolean;
}

export interface TreeEntry {
  mode: string;
  oid: string;
}

export interface DiffEntry {
  status: string;
  path: string;
  old_path: string | null;
}

export interface FileView {
  exists: boolean;
  binary: boolean;
  size: number;
  oid: string | null;
  text: string | null;
  truncated: boolean;
}

export interface ApplyPlan {
  changes: { path: string; action: { kind: string } }[];
  conflicts: { path: string; reason: string }[];
  skipped: { path: string; reason: string }[];
  head: { kind: string; branch?: string };
  index_tree: string | null;
  local_changes: boolean;
  ours_branch: string | null;
  theirs_branch: string | null;
  needs_branch_confirmation: boolean;
  blockers: string[];
  notes: string[];
}

export interface ApplyPreview {
  checkpoint: Checkpoint;
  plan: ApplyPlan;
  auto_eligible: boolean;
  ide: { safe: boolean; running: string[]; reason: string };
  origin_verified: boolean;
  device_approved: boolean;
}

export type ApplyOutcome =
  | { outcome: "applied"; result: ApplyResult; safety_checkpoint: string; lineage_head: string }
  | { outcome: "nothing_to_do"; safety_checkpoint: string }
  | { outcome: "conflicts"; conflict_id: string; count: number }
  | { outcome: "blocked"; plan: ApplyPlan }
  | { outcome: "needs_manual"; reason: string };

export interface ApplyResult {
  written: string[];
  deleted: string[];
  preserved: string[];
  preserved_dir: string | null;
  integrity_failures: string[];
  head_updated: boolean;
  index_updated: boolean;
}

export interface ConflictRow {
  id: string;
  project_id: string;
  checkpoint_id: string;
  safety_checkpoint: string;
  state: string;
  created_ms: number;
}

export interface ConflictDetail {
  conflict: ConflictRow;
  theirs_device: string;
  files: { path: string; reason: string; ours: FileView; theirs: FileView; base: FileView }[];
}

export interface Activity {
  id: number;
  project_id: string | null;
  ts_ms: number;
  level: string;
  kind: string;
  message: string;
}

export interface Device {
  id: string;
  name: string;
  first_seen_ms: number;
  last_seen_ms: number;
  is_local: boolean;
  public_key: string | null;
  fingerprint: string | null;
  approved: boolean;
}

export interface Finding {
  checkpoint_id: string;
  fingerprint: string;
  path: string;
  line: number | null;
  rule: string;
  severity: "blocking" | "warning";
  excerpt: string;
  allowed: boolean;
}

export interface Settings {
  default_interval_secs: number;
  fetch_interval_secs: number;
  quiet_secs: number;
  autostart: boolean;
  github_client_id: string;
  retention: { keep_all_hours: number; daily_days: number; weekly_weeks: number; min_keep: number };
  prune_remote: boolean;
}

export type Side = { kind: "checkpoint"; id: string } | { kind: "head" } | { kind: "current" };
