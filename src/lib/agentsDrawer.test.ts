import { describe, expect, test } from "bun:test";

import type { ApprovalRequest, FileDiff, SubagentRun, TurnDigest } from "./ipc";
import {
  drawerPill,
  drawerTab,
  hasAgentDrawer,
  pendingSubagentApprovals,
  pillOpensTab,
  pillTurn,
  pillTurnCount,
  pillTurnHead,
  pillTurnLabel,
  queueGoesToAgents,
  roundProgress,
  splitPath,
  TURN_ROWS,
  turnAgo,
  turnBand,
  turnMeta,
  turnRows,
  turnTabCount,
} from "./agentsDrawer";

const run = (
  id: string,
  status: SubagentRun["status"],
  round = 1,
  over: Partial<SubagentRun> = {},
): SubagentRun => ({
  agent_id: id,
  agent_type: "general-purpose",
  description: id,
  status,
  started_at_ms: 0,
  ended_at_ms: status === "done" ? 1 : null,
  summary: null,
  round,
  interrupted: false,
  ...over,
});

const approval = (
  id: number,
  agent: string | null,
  session = "s1",
): ApprovalRequest => ({
  id,
  session_id: session,
  command: "bun run test:fixtures --update",
  cwd: null,
  risk: "yellow",
  risk_reason: null,
  context: null,
  requested_at_ms: id,
  agent_id: agent,
});

describe("roundProgress", () => {
  test("conta concluídos / total só da rodada atual", () => {
    const subs = [
      run("a", "done", 1),
      run("b", "done", 1),
      run("c", "running", 2),
      run("d", "done", 2),
      run("e", "starting", 2),
    ];
    expect(roundProgress(subs)).toEqual({
      done: 1,
      total: 3,
      running: true,
      interrupted: 0,
    });
  });

  test("interrompido não conta como concluído (4/5 · 1 interrompido)", () => {
    const subs = [
      run("a", "done"),
      run("b", "done"),
      run("c", "done"),
      run("d", "done"),
      run("e", "done", 1, { interrupted: true }),
    ];
    expect(roundProgress(subs)).toEqual({
      done: 4,
      total: 5,
      running: false,
      interrupted: 1,
    });
  });

  test("rodada assentada: algum terminou e nenhum roda (starting não segura)", () => {
    const subs = [run("a", "done"), run("ghost", "starting")];
    expect(roundProgress(subs)?.running).toBe(false);
    expect(roundProgress([run("a", "starting")])?.running).toBe(true);
  });

  test("sem subagentes não há contagem", () => {
    expect(roundProgress([])).toBeNull();
  });
});

describe("pendingSubagentApprovals", () => {
  test("conta os pedidos dos subagentes desta sessão, vermelho incluso", () => {
    const subs = [run("a", "running"), run("b", "running")];
    const approvals = [
      approval(1, "a"),
      { ...approval(2, "b"), risk: "red" as const },
      approval(3, null), // agente principal
      approval(4, "a", "s2"), // outra sessão
      approval(5, "zzz"), // não é subagente daqui
    ];
    expect(pendingSubagentApprovals("s1", subs, approvals)).toBe(2);
  });
});

describe("drawerPill", () => {
  const subs = [run("a", "done"), run("b", "running"), run("c", "done")];

  test("só fila: sessão sem subagentes", () => {
    expect(drawerPill([], 0, false)).toBeNull();
  });

  test("rodada em curso", () => {
    expect(drawerPill(subs, 0, false)).toEqual({
      done: 2,
      total: 3,
      state: { kind: "run" },
    });
  });

  test("aprovação pendente pede atenção, acima de rodando", () => {
    expect(drawerPill(subs, 2, false)?.state).toEqual({
      kind: "attn",
      count: 2,
    });
  });

  test("rodada terminou: ✓ até o dono abrir", () => {
    const done = [run("a", "done"), run("b", "done")];
    expect(drawerPill(done, 0, false)).toEqual({
      done: 2,
      total: 2,
      state: { kind: "ok" },
    });
    expect(drawerPill(done, 0, true)).toEqual({
      done: 2,
      total: 2,
      state: null,
    });
  });

  test("terminou com interrupção mostra o vermelho no lugar do ✓", () => {
    const failed = [run("a", "done"), run("b", "done", 1, { interrupted: true })];
    expect(drawerPill(failed, 0, false)?.state).toEqual({
      kind: "fail",
      count: 1,
    });
    expect(drawerPill(failed, 0, true)?.state).toBeNull();
  });
});

describe("drawerTab", () => {
  const base = { owner: null, hasSubagents: true, pending: 0, running: false };

  test("sem subagentes não há aba de agentes: é a fila", () => {
    expect(drawerTab({ ...base, hasSubagents: false, owner: "agents" })).toBe(
      "queue",
    );
  });

  test("aprovação pendente abre em Agentes", () => {
    expect(drawerTab({ ...base, pending: 1 })).toBe("agents");
  });

  test("Agentes durante a rodada, Fila fora dela", () => {
    expect(drawerTab({ ...base, running: true })).toBe("agents");
    expect(drawerTab(base)).toBe("queue");
  });

  test("a escolha do dono vale (até recolher, quando o core a zera)", () => {
    expect(drawerTab({ ...base, running: true, owner: "queue" })).toBe("queue");
    expect(drawerTab({ ...base, pending: 2, owner: "queue" })).toBe("queue");
    expect(drawerTab({ ...base, owner: "agents" })).toBe("agents");
  });
});

describe("aba Turno: quem escolhe (T1 e T2 de ResumoDoTurno)", () => {
  const base = { owner: null, hasSubagents: false, pending: 0, running: false };
  const done = { last_turn: { seq: 1 } as TurnDigest, turn_open: false };
  const open = { ...done, turn_open: true };
  const click = { hasSubagents: false, pending: 0, running: false, queuePaused: false };

  test("a gaveta aberta nunca troca para Turno sozinha", () => {
    // O turno terminou com o dono escrevendo na Fila: continua na Fila.
    expect(drawerTab({ ...base, turn: done })).toBe("queue");
    expect(drawerTab({ ...base, hasSubagents: true, turn: done })).toBe("queue");
  });

  test("a aba Turno é escolha do dono e só existe com turno terminado", () => {
    expect(drawerTab({ ...base, turn: done, owner: "turn" })).toBe("turn");
    // Turno novo aberto: a aba continua, mostrando o anterior.
    expect(drawerTab({ ...base, turn: open, owner: "turn" })).toBe("turn");
    expect(drawerTab({ ...base, owner: "turn" })).toBe("queue");
    expect(drawerTab({ ...base, owner: "turn", turn: { last_turn: null } })).toBe("queue");
  });

  test("clicar na pílula depois de um turno terminado abre em Turno", () => {
    expect(pillOpensTab({ ...click, turn: done })).toBe("turn");
    expect(pillOpensTab({ ...click, hasSubagents: true, turn: done })).toBe("turn");
  });

  test("o que pede você vence o turno: abre pela regra", () => {
    const agents = { ...click, hasSubagents: true, turn: done };
    expect(pillOpensTab({ ...agents, pending: 1 })).toBeNull();
    expect(pillOpensTab({ ...agents, running: true })).toBeNull();
    expect(pillOpensTab({ ...click, queuePaused: true, turn: done })).toBeNull();
  });

  test("sem turno terminado, ou com turno novo aberto, abre pela regra", () => {
    expect(pillOpensTab({ ...click, turn: open })).toBeNull();
    expect(pillOpensTab({ ...click, turn: { last_turn: null, turn_open: false } })).toBeNull();
    expect(pillOpensTab({ ...click, turn: null })).toBeNull();
  });
});

describe("queueGoesToAgents", () => {
  test("fila pausada por aprovação de subagente aponta para Agentes", () => {
    expect(queueGoesToAgents("approval", 1)).toBe(true);
  });

  test("aprovação do agente principal, ou outra pausa, não aponta", () => {
    expect(queueGoesToAgents("approval", 0)).toBe(false);
    expect(queueGoesToAgents("user", 1)).toBe(false);
    expect(queueGoesToAgents(null, 1)).toBe(false);
  });
});

describe("hasAgentDrawer (qual sessão a pílula mostra)", () => {
  test("sessão de agente sempre tem gaveta", () => {
    expect(hasAgentDrawer("agent", false)).toBe(true);
  });
  test("shell só com claude cru achado pelo probe", () => {
    expect(hasAgentDrawer("shell", true)).toBe(true);
    expect(hasAgentDrawer("shell", false)).toBe(false);
  });
  test("ssh e container não têm", () => {
    expect(hasAgentDrawer("ssh", true)).toBe(false);
    expect(hasAgentDrawer("container", true)).toBe(false);
  });
});

describe("pillTurn (trecho do turno na pílula, quadro Pílulas de ResumoDoTurno)", () => {
  // Tradutor falso com os mesmos textos pt-BR do i18n — o teste é da montagem.
  const PT: Record<string, string> = {
    pillTurn: "Turno",
    pillTurnFailed: "Turno falhou",
    pillTurnInterrupted: "Turno interrompido",
    pillTurnNoChanges: "sem mudanças",
    pillTurnNoGit: "sem git",
    diffFilesSummary_one: "{{count}} arquivo",
    diffFilesSummary_other: "{{count}} arquivos",
  };
  const t = (key: string, options: Record<string, unknown> = {}) =>
    (PT[key] ?? PT[`${key}_${options.count === 1 ? "one" : "other"}`] ?? key).replace(
      /\{\{(\w+)\}\}/g,
      (_, k) => String(options[k]),
    );

  const digest = (over: Partial<TurnDigest> = {}): TurnDigest => ({
    seq: 1,
    outcome: "completed",
    failure: null,
    started_at_ms: 0,
    ended_at_ms: 1,
    duration_ms: 1,
    prompt: null,
    diff: "ok",
    files: [],
    files_total: 0,
    insertions: 0,
    deletions: 0,
    commits: 0,
    cost_usd: null,
    tokens: null,
    ...over,
  });
  const seg = (over: Partial<TurnDigest> = {}) => {
    const turn = pillTurn({ last_turn: digest(over), turn_open: false });
    if (!turn) throw new Error("sem trecho");
    return turn;
  };
  const shown = (over: Partial<TurnDigest> = {}) => {
    const turn = seg(over);
    return [pillTurnHead(turn, t), pillTurnLabel(turn, t), pillTurnCount(turn)];
  };

  test("sem resumo não há trecho", () => {
    expect(pillTurn(undefined)).toBeNull();
    expect(pillTurn({})).toBeNull();
    expect(pillTurn({ last_turn: null, turn_open: false })).toBeNull();
  });

  test("turno aberto: o trecho some, e volta quando ele termina", () => {
    const last_turn = digest({ files_total: 6, insertions: 115, deletions: 55 });
    expect(pillTurn({ last_turn, turn_open: true })).toBeNull();
    expect(pillTurn({ last_turn, turn_open: false })).not.toBeNull();
    expect(pillTurn({ last_turn })).not.toBeNull();
  });

  test("terminou e mexeu em arquivo", () => {
    const over = { files_total: 6, insertions: 115, deletions: 55 };
    expect(seg(over)).toEqual({
      outcome: "completed",
      body: { kind: "counts", files: 6, insertions: 115, deletions: 55 },
    });
    expect(shown(over)).toEqual(["Turno", "Turno · 6 arquivos +115 −55", 6]);
  });

  test("um arquivo fica no singular", () => {
    expect(shown({ files_total: 1, insertions: 9, deletions: 9 })).toEqual([
      "Turno",
      "Turno · 1 arquivo +9 −9",
      1,
    ]);
  });

  test("os números são os totais do resumo, não o tamanho da lista cortada", () => {
    const over = { files: [], files_total: 230, insertions: 1204, deletions: 388 };
    expect(seg(over).body).toEqual({
      kind: "counts",
      files: 230,
      insertions: 1204,
      deletions: 388,
    });
  });

  test("terminou sem mexer em nada", () => {
    expect(seg().body).toEqual({ kind: "none" });
    expect(shown()).toEqual(["Turno", "Turno · sem mudanças", 0]);
  });

  test("você interrompeu", () => {
    expect(
      shown({ outcome: "interrupted", files_total: 3, insertions: 40, deletions: 6 }),
    ).toEqual(["Turno interrompido", "Turno interrompido · 3 arquivos +40 −6", 3]);
  });

  test("turno falhou, com e sem arquivos", () => {
    const failed = { outcome: "failed" as const, failure: "limite de uso (429)" };
    expect(seg(failed).outcome).toBe("failed");
    expect(shown({ ...failed, files_total: 2, insertions: 14, deletions: 3 })).toEqual([
      "Turno falhou",
      "Turno falhou · 2 arquivos +14 −3",
      2,
    ]);
    expect(shown(failed)).toEqual(["Turno falhou", "Turno falhou · sem mudanças", 0]);
  });

  test("pasta sem git", () => {
    expect(seg({ diff: "no_repo" }).body).toEqual({ kind: "no_repo" });
    expect(shown({ diff: "no_repo" })).toEqual(["Turno", "Turno · sem git", 0]);
  });

  test("snapshot indisponível: só o rótulo, sem contagem e sem falar de git", () => {
    // Contagens que por acaso vierem não aparecem: o core não mediu.
    const over = { diff: "unavailable" as const, files_total: 4, insertions: 1 };
    expect(seg(over).body).toEqual({ kind: "unavailable" });
    expect(shown(over)).toEqual(["Turno", "Turno", null]);
    expect(shown({ ...over, outcome: "failed" })).toEqual(["Turno falhou", "Turno falhou", null]);
  });
});

describe("aba Turno (quadros T2 e T3 de ResumoDoTurno)", () => {
  const PT: Record<string, string> = {
    agentsAgo: "há {{time}}",
    agentsRoundMinutes: "há {{count}} min",
    agentsRoundHours: "há {{count}} h",
    agentsUsageTokens_one: "{{tokens}} token",
    agentsUsageTokens_other: "{{tokens}} tokens",
    turnCommits_one: "{{count}} commit",
    turnCommits_other: "{{count}} commits",
  };
  const t = (key: string, options: Record<string, unknown> = {}) =>
    (PT[key] ?? PT[`${key}_${options.count === 1 ? "one" : "other"}`] ?? key).replace(
      /\{\{(\w+)\}\}/g,
      (_, k) => String(options[k]),
    );
  const digest = (over: Partial<TurnDigest> = {}): TurnDigest => ({
    seq: 1,
    outcome: "completed",
    failure: null,
    started_at_ms: 0,
    ended_at_ms: 1,
    duration_ms: 1,
    prompt: null,
    diff: "ok",
    files: [],
    files_total: 0,
    insertions: 0,
    deletions: 0,
    commits: 0,
    cost_usd: null,
    tokens: null,
    ...over,
  });
  const file = (path: string): FileDiff => ({
    path,
    old_path: null,
    status: "modified",
    insertions: 1,
    deletions: 0,
  });

  test("faixa: falha e interrupção vencem o que o diff mediu", () => {
    expect(turnBand(digest({ files_total: 2 }))).toBe("changed");
    expect(turnBand(digest())).toBe("none");
    expect(turnBand(digest({ diff: "no_repo" }))).toBe("no_repo");
    expect(turnBand(digest({ diff: "unavailable" }))).toBe("unavailable");
    expect(turnBand(digest({ outcome: "failed", files_total: 2 }))).toBe("failed");
    expect(turnBand(digest({ outcome: "interrupted", diff: "no_repo" }))).toBe(
      "interrupted",
    );
  });

  test("lista: 8 linhas e o resto pelo total do resumo, não pelo tamanho da lista", () => {
    const files = Array.from({ length: 200 }, (_, i) => file(`src/f${i}.rs`));
    const { rows, more } = turnRows(digest({ files, files_total: 230 }));
    expect(rows).toHaveLength(TURN_ROWS);
    expect(more).toBe(222);
    expect(turnRows(digest({ files: files.slice(0, 3), files_total: 3 }))).toEqual({
      rows: files.slice(0, 3),
      more: 0,
    });
  });

  test("contagem da aba: arquivos, ou traço sem nenhum", () => {
    expect(turnTabCount(digest({ files_total: 6 }))).toBe(6);
    expect(turnTabCount(digest())).toBeNull();
    expect(turnTabCount(digest({ diff: "no_repo" }))).toBeNull();
  });

  test("segunda linha: só o que o core mandou", () => {
    expect(
      turnMeta(
        digest({ duration_ms: 291_000, tokens: 65_400, cost_usd: 0.41, commits: 1 }),
        "en",
        t,
      ),
    ).toBe("4m 51s · 65.4K tokens · $0.41 · 1 commit");
    // Codex: sem custo nem tokens.
    expect(turnMeta(digest({ duration_ms: 108_000 }), "en", t)).toBe("1m 48s");
    expect(turnMeta(digest({ duration_ms: 48_000, commits: 3 }), "en", t)).toBe(
      "48s · 3 commits",
    );
  });

  test("há quanto tempo terminou: segundos, minutos, horas", () => {
    const turn = digest({ ended_at_ms: 1_000_000 });
    expect(turnAgo(turn, 1_008_000, t)).toBe("há 8s");
    expect(turnAgo(turn, 1_000_000 + 3 * 60_000, t)).toBe("há 3 min");
    expect(turnAgo(turn, 1_000_000 + 2 * 3_600_000, t)).toBe("há 2 h");
    // Relógio do webview atrás do core: nunca negativo.
    expect(turnAgo(turn, 999_000, t)).toBe("há 0s");
  });

  test("caminho: pasta com a barra e o nome", () => {
    expect(splitPath("src-tauri/src/status/osc133.rs")).toEqual({
      dir: "src-tauri/src/status/",
      name: "osc133.rs",
    });
    expect(splitPath("README.md")).toEqual({ dir: "", name: "README.md" });
  });
});
