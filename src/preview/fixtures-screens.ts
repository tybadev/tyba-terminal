// Dados das telas cheias (Git, Arquivos, Docker, gaveta de agentes), copiados
// dos quadros GitPRs, GitPrDetail, GitCI, Files, Docker e Agents. Sintéticos.

import type {
  ContainerInfo,
  DirListing,
  FileDecoration,
  HandoffTarget,
  LogLine,
  PrDetail,
  PrListItem,
  PrPage,
  RunLog,
  SubagentRun,
  WorkflowJob,
  WorkflowRun,
} from "../lib/ipc";
import type { Graph } from "../lib/orbit";
import { ID, REPO } from "./fixtures";

const NOW = Date.now();
const ago = (ms: number) => new Date(NOW - ms).toISOString();
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

export const BRANCH = "feat/handoff-agentes-ui";
const REPO_SLUG = "exemplo/tyba-terminal";
const prUrl = (n: number) => `https://github.com/${REPO_SLUG}/pull/${n}`;

function pr(over: Partial<PrListItem> & Pick<PrListItem, "number" | "title" | "head">): PrListItem {
  return {
    url: prUrl(over.number),
    state: "open",
    author: "gteodoro",
    author_name: null,
    base: "main",
    updated_at: ago(12 * MIN),
    merged_at: null,
    merged_by: null,
    ci: { passed: 5, failed: 0, pending: 0, total: 5 },
    review: null,
    approvals: 0,
    review_requested: false,
    comments: 0,
    additions: 0,
    deletions: 0,
    cross_repo: false,
    session: null,
    ...over,
  };
}

const core = { session: ID.core, title: "core", agent: "claude" };
const api = { session: ID.api, title: "api", agent: "codex" };

export const PR_PAGE: PrPage = {
  items: [
    pr({
      number: 437,
      title: "feat(ui): handoff entre agentes na aba Turno, folha e paleta",
      head: BRANCH,
      review: "review_required",
      review_requested: true,
      comments: 2,
      additions: 412,
      deletions: 38,
      session: core,
    }),
    pr({
      number: 436,
      title: "feat(agent): handoff entre agentes no core",
      head: "feat/handoff-agentes",
      updated_at: ago(40 * MIN),
      review: "approved",
      approvals: 1,
      comments: 4,
      additions: 286,
      deletions: 12,
      session: core,
    }),
    pr({
      number: 438,
      title: "fix(sidebar): grupo SSH some ao renomear host",
      head: "fix/ssh-grupo",
      updated_at: ago(2 * HOUR),
      ci: { passed: 3, failed: 0, pending: 2, total: 5 },
      additions: 24,
      deletions: 6,
      session: api,
    }),
    pr({
      number: 431,
      title: "refactor(pty): batching por janela de 12 ms",
      head: "refactor/pty-batch",
      state: "draft",
      author: "mariasouza",
      updated_at: ago(DAY),
      ci: { passed: 4, failed: 1, pending: 0, total: 5 },
      comments: 1,
      additions: 98,
      deletions: 140,
    }),
    pr({
      number: 439,
      title: "chore(deps): bump Swatinem/rust-cache",
      head: "dependabot/rust-cache",
      author: "dependabot",
      updated_at: ago(3 * DAY),
      ci: { passed: 4, failed: 1, pending: 0, total: 5 },
      review: "changes_requested",
      comments: 1,
      additions: 2,
      deletions: 2,
    }),
  ],
  recent_merged: [
    pr({
      number: 435,
      title: "feat(notifications): avisos do turno na Inbox",
      head: "feat/resumo-turno-avisos",
      state: "merged",
      merged_at: ago(DAY),
      merged_by: "gteodoro",
      updated_at: ago(DAY),
      approvals: 1,
      comments: 3,
      additions: 188,
      deletions: 21,
      session: core,
    }),
    pr({
      number: 434,
      title: "feat(git): diff do turno no resumo",
      head: "feat/resumo-turno-diff",
      state: "merged",
      merged_at: ago(DAY),
      merged_by: "gteodoro",
      updated_at: ago(DAY),
      approvals: 1,
      comments: 1,
      additions: 240,
      deletions: 33,
      session: core,
    }),
  ],
  counts: { open: 5, mine: 4, review: 1, draft: 1 },
  capped: false,
  repo: REPO_SLUG,
  forge: "github",
  fetched_at: new Date(NOW).toISOString(),
  rate_limit: null,
};

const check = (name: string, secs: number) => ({
  name,
  workflow: "CI",
  status: "success" as const,
  started_at: ago(DAY + secs * 1000),
  completed_at: ago(DAY),
  url: null,
});

export const PR_DETAIL: PrDetail = {
  number: 437,
  title: "feat(ui): handoff entre agentes na aba Turno, folha e paleta",
  url: prUrl(437),
  state: "open",
  author: "gteodoro",
  head: BRANCH,
  base: "feat/handoff-agentes",
  head_sha: "c41d9e7a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e",
  created_at: ago(2 * DAY),
  body:
    'Liga o handoff do core (#436) na interface: botão "Passar para…" na aba Turno, folha com a nota e o agente de destino, e a entrada na paleta.',
  additions: 412,
  deletions: 38,
  changed_files: 9,
  commits_total: 6,
  commits: [
    { sha: "8f2c1a0b", subject: "feat(ui): folha do handoff com nota", author: "gteodoro", authored_at: ago(DAY) },
    { sha: "c41d9e7a", subject: "feat(ui): Passar para… na paleta", author: "gteodoro", authored_at: ago(DAY) },
  ],
  files: [
    { path: "src/components/HandoffSheet.tsx", additions: 214, deletions: 0 },
    { path: "src/components/TurnPanel.tsx", additions: 42, deletions: 7 },
    { path: "src/components/CommandPalette.tsx", additions: 18, deletions: 3 },
  ],
  checks: [check("lint", 42), check("test (macos)", 182), check("test (linux)", 168), check("build", 70)],
  reviews: [
    {
      author: "mariasouza",
      state: "commented",
      submitted_at: ago(3 * HOUR),
      body: "A folha fecha sozinha se eu clicar fora sem querer. Dá para pedir confirmação quando já tem nota escrita?",
      commit: null,
    },
  ],
  pending_reviewers: [],
  threads: [],
  comments: [],
  review: "review_required",
  mergeable: "mergeable",
  merge_state: "blocked",
  cross_repo: false,
  session: { session: ID.claudeApi, title: "api", agent: "claude" },
  truncated: false,
  rate_limit: null,
};

const run = (id: number, name: string, event: string, status: string, conclusion: string | null, age: number): WorkflowRun => ({
  id,
  name,
  status,
  conclusion,
  url: `https://github.com/${REPO_SLUG}/actions/runs/${id}`,
  head_branch: BRANCH,
  event,
  created_at: ago(age),
});

export const WORKFLOW_RUNS: WorkflowRun[] = [
  run(905, "CI", "pull_request", "in_progress", null, 10_000),
  run(904, "CI", "push", "completed", "failure", 5 * MIN),
  run(903, "Release", "push", "completed", "success", 2 * HOUR),
  run(902, "CI", "push", "completed", "success", DAY),
  run(901, "Docs", "pull_request", "completed", "success", 3 * DAY),
];

const job = (id: number, name: string, conclusion: string, secs: number, steps: [string, string][] = []): WorkflowJob => ({
  id,
  name,
  status: "completed",
  conclusion,
  url: null,
  started_at: ago(5 * MIN + secs * 1000),
  completed_at: ago(5 * MIN),
  steps: steps.map(([n, c]) => ({ name: n, status: "completed", conclusion: c })),
});

export const WORKFLOW_JOBS: WorkflowJob[] = [
  job(1, "lint", "success", 42),
  job(2, "test (macos)", "failure", 187, [
    ["Set up job", "success"],
    ["Checkout", "success"],
    ["Install Rust", "success"],
    ["Cache cargo", "success"],
    ["cargo test --lib", "failure"],
    ["Post checkout", "success"],
  ]),
  job(3, "test (linux)", "success", 171),
  job(4, "build", "success", 72),
];

const LOG = [
  "running 214 tests",
  "test agent::handoff::tests::builds_prompt_from_turn ... ok",
  "test agent::handoff::tests::keeps_base_sha ... ok",
  "test agent::handoff::tests::redacts_secrets_in_note ... FAILED",
  "test session::store::tests::roundtrip ... ok",
  "test status::osc133::tests::parses_prompt_mark ... ok",
  "",
  "failures:",
  "",
  "---- agent::handoff::tests::redacts_secrets_in_note stdout ----",
  "thread 'redacts_secrets_in_note' panicked at src/agent/handoff.rs:212:9:",
  "assertion `left == right` failed",
  '  left: "chave [redigido] no fim"',
  ' right: "chave [redigido]"',
  "",
  "test result: FAILED. 213 passed; 1 failed; 0 ignored",
  "error: test failed, to rerun pass `--lib`",
  "##[error]Process completed with exit code 101.",
];

export const RUN_LOG: RunLog = {
  text: LOG.join("\n"),
  truncated: false,
  failed_step: "cargo test --lib",
  error_line: LOG.length - 1,
};

// --- Arquivos ---------------------------------------------------------------

const entry = (rel: string, isDir = false, gitignored = false) => ({
  name: rel.split("/").pop() ?? rel,
  rel_path: rel,
  is_dir: isDir,
  is_symlink: false,
  gitignored,
});

const DIRS: Record<string, ReturnType<typeof entry>[]> = {
  "": [
    entry("src", true),
    entry("src-tauri", true),
    entry("docs", true),
    entry("node_modules", true, true),
    entry(".env.local", false, true),
    entry("CLAUDE.md"),
    entry("package.json"),
  ],
  src: [entry("src/components", true), entry("src/lib", true), entry("src/i18n", true), entry("src/App.tsx")],
  "src/components": [
    entry("src/components/AgentsPanel.tsx"),
    entry("src/components/HandoffSheet.tsx"),
    entry("src/components/TurnPanel.tsx"),
    entry("src/components/CommandPalette.tsx"),
  ],
};

export function listDir(path: string): DirListing {
  const entries = DIRS[path] ?? [];
  return { dir: path, entries, total: entries.length, truncated: false };
}

export const DECORATIONS: FileDecoration[] = [
  { path: "src/components/HandoffSheet.tsx", status: "added" },
  { path: "src/components/TurnPanel.tsx", status: "modified" },
  { path: "src/components/CommandPalette.tsx", status: "modified" },
  { path: "CLAUDE.md", status: "modified" },
];

export const TURN_PANEL_SRC = `import { useState } from "react";

export function TurnPanel({ turn, onHandoff }: Props) {
  const files = useTurnFiles(turn.id);
  const [sheet, setSheet] = useState(false);

  return (
    <section className="turn">
      <TurnSummary turn={turn} files={files} />
      <Button onClick={() => setSheet(true)}>
        {t("turn.handoff")}
      </Button>
      {sheet && (
        <HandoffSheet turn={turn} onClose={() => setSheet(false)} />
      )}
    </section>
  );
}
`;

// --- Docker -----------------------------------------------------------------

function container(
  id: string,
  name: string,
  image: string,
  state: string,
  status: string,
  ports = "",
  service = name,
): ContainerInfo {
  return {
    id,
    name,
    image,
    state,
    status,
    ports,
    compose_project: "api",
    compose_working_dir: REPO.api,
    service,
    config_files: `${REPO.api}/docker-compose.yml`,
    working_dir_status: "ok",
    compose_file_status: "ok",
    in_repo: true,
  };
}

export const CONTAINERS: ContainerInfo[] = [
  container("c-postgres18", "muriki_postgres", "muriki-postgres:18", "exited", "Exited (0) 5 days ago", "", "postgres"),
  container("c-redis", "redis", "redis:7-alpine", "exited", "Exited (0) 5 days ago"),
  container("c-postgres", "postgres", "postgres:16", "running", "Up 34 hours (healthy)", "0.0.0.0:7432->5432/tcp"),
  container("c-greenmail", "greenmail", "greenmail/standalone", "exited", "Exited (255) 5 weeks ago"),
  container("c-n8n", "n8n", "n8nio/n8n", "exited", "Exited (255) 5 weeks ago"),
  container("c-rabbit", "rabbitmq", "rabbitmq:3.13-management", "exited", "Exited (0) 9 days ago"),
  // O resto dos 25 do quadro ("4 rodando · 21 parados"), abaixo da dobra.
  ...["minio", "mailpit", "keycloak"].map((n) =>
    container(`c-${n}`, n, `${n}:latest`, "running", "Up 2 days"),
  ),
  ...Array.from({ length: 16 }, (_, i) =>
    container(`c-job${i}`, `job-${i + 1}`, "alpine:3.20", "exited", "Exited (0) 6 weeks ago"),
  ),
];

/** RAM do postgres rodando (quadro: "saudável · RAM 63 MB · há 34 h"). */
export const PANEL_USAGE = {
  usage: [{ id: "c-postgres", cpu_percent: 0.4, mem_bytes: 63 * 1024 * 1024 }],
  exits: [],
};

const PG_LOG: [string, string, boolean][] = [
  ["04:28:36", '[1] LOG:  background worker "logical replication launcher" (PID 36) exited with code 1', false],
  ["04:28:36", "[30] LOG:  shutting down", false],
  ["04:28:36", "[30] LOG:  checkpoint complete: wrote 0 buffers (0.0%); write=0.001 s, sync=0.001 s", false],
  ["11:28:46", "[1] LOG:  starting PostgreSQL 18.6 on aarch64-unknown-linux-gnu", false],
  ["11:28:46", '[1] LOG:  listening on IPv4 address "0.0.0.0", port 5432', false],
  ["11:28:46", "[1] LOG:  database system is ready to accept connections", false],
  ["13:39:37", "[1] FATAL:  terminating connection due to administrator command", true],
  ["13:39:37", "[30] LOG:  checkpoint starting: shutdown immediate", false],
  ["13:39:37", "[1] LOG:  database system is shut down", false],
];

function clockToday(clock: string): number {
  const [h, m, sec] = clock.split(":").map(Number);
  const d = new Date(NOW);
  d.setHours(h, m, sec, 0);
  return d.getTime();
}

export const CONTAINER_LOG: LogLine[] = PG_LOG.map(([ts, text, error], seq) => ({
  seq,
  ts,
  ts_ms: clockToday(ts),
  level: error ? "error" : "info",
  continuation: false,
  stderr: error,
  text,
  frame: null,
}));

// --- Agentes e handoff ------------------------------------------------------

const sub = (id: string, description: string, tool: string, detail: string, secs: number): SubagentRun => ({
  agent_id: id,
  agent_type: "general-purpose",
  description,
  status: "running",
  started_at_ms: NOW - secs * 1000,
  ended_at_ms: null,
  summary: null,
  interrupted: false,
  round: 1,
  activity_tool: tool,
  activity_detail: detail,
  activity_at_ms: NOW - 1000,
  recent_activity: [],
});

export const SUBAGENTS: Record<string, SubagentRun[]> = {
  [ID.claudeApi]: [
    sub("sa-sleep", "Run sleep 2", "Bash", 'sleep 2 && echo "sleep 2 concluído"', 6),
    sub("sa-for", "Run for loop 1 to 10", "Bash", "for i in $(seq 1 10); do echo $i; done", 7),
    sub("sa-ls", "Run ls -la", "Bash", "ls -la", 8),
  ],
  [ID.tyba]: [{ ...sub("sa-explore", "achar o TurnPanel", "Grep", "TurnPanel", 40), agent_type: "Explore" }],
};

export const HANDOFF_TARGETS: HandoffTarget[] = [
  { agent: "codex", label: "Codex", available: true, reason: null },
  { agent: "gemini", label: "Gemini", available: false, reason: "sem suporte ainda" },
];

// --- Orbit ------------------------------------------------------------------

/** O grafo do quadro Orbit: o Terminal · api, o Revisor e o plano dele. */
export const ORBIT_GRAPH: Graph = {
  id: "graph-teste",
  name: "teste grafo",
  dir: REPO.api,
  nodes: [
    { id: "n-term", title: "Terminal · api", kind: { type: "terminal", dir: REPO.api }, position: { x: 20, y: 40 }, size: { w: 380, h: 300 } },
    {
      id: "n-rev",
      title: "Revisor",
      kind: { type: "agent", runner: "claude", role: "Revisor", prompt: "Revise o billing.", dir: REPO.api },
      position: { x: 440, y: 40 },
      size: { w: 360, h: 300 },
    },
    {
      id: "n-plan",
      title: "Plano",
      kind: { type: "note", markdown: "- [x] Ler o contexto\n- [ ] Rodar a suíte" },
      position: { x: 440, y: 380 },
      size: { w: 300, h: 140 },
    },
  ],
  edges: [],
  variables: {},
};
