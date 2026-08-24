import { describe, expect, test } from "bun:test";

import type {
  LayoutState,
  PaneNode,
  Session,
  SessionKind,
  SessionStatus,
  Tab,
  Workspace,
} from "./ipc";
import {
  buildRows,
  groupByWorkspace,
  nextAttention,
  placesBySession,
  urgencyOf,
  wantsAttention,
} from "./agentsBoard";

const session = (
  id: string,
  status: SessionStatus,
  over: Partial<Session> = {},
): Session =>
  ({
    id,
    kind: { type: "agent", runner: "claude_code" } as SessionKind,
    title: id,
    repo_root: null,
    worktree: null,
    status,
    attention: false,
    created_at: "",
    ...over,
  }) as Session;

const leaf = (paneId: string, sessionId: string): PaneNode => ({
  type: "leaf",
  id: paneId,
  session_id: sessionId,
});

const tab = (id: string, root: PaneNode | null): Tab =>
  ({
    id,
    title: null,
    view: null,
    active_pane: null,
    root,
    created_at: "",
  }) as Tab;

const workspace = (
  id: string,
  name: string,
  tabs: Tab[],
  over: Partial<Workspace> = {},
): Workspace =>
  ({
    id,
    name,
    name_locked: false,
    repo_root: null,
    color: null,
    group: null,
    kind: "user",
    launch_config_id: null,
    active_tab: null,
    tabs,
    side_view: null,
    side_ratio: 0.5,
    side_expanded: false,
    created_at: "",
    ...over,
  }) as Workspace;

const layout = (workspaces: Workspace[]): LayoutState => ({
  workspaces,
  active_workspace: workspaces[0]?.id ?? null,
});

const blocked: SessionStatus = {
  state: "awaiting_input",
  hint: null,
  reason: "approval",
};
const asking: SessionStatus = {
  state: "awaiting_input",
  hint: null,
  reason: "reply",
};
const running: SessionStatus = { state: "running" };
const failed: SessionStatus = { state: "failed", reason: "boom" };
const done: SessionStatus = { state: "idle", summary: null };

describe("urgência", () => {
  test("aprovação vem antes de resposta, mesmo empatadas no rank do sidebar", () => {
    // As duas são `awaiting_input` e valem rank 3 no sidebar, onde a linha
    // mostra o motivo. No quadro, quem ordena é a urgência.
    expect(urgencyOf(session("a", blocked))).toBeGreaterThan(
      urgencyOf(session("b", asking)),
    );
  });

  test("a ordem entre estados é falha > bloqueado > concluído > rodando", () => {
    const ranked = [
      urgencyOf(session("a", failed)),
      urgencyOf(session("b", blocked)),
      urgencyOf(session("c", done, { attention: true })),
      urgencyOf(session("d", running)),
    ];
    expect(ranked).toEqual([...ranked].sort((x, y) => y - x));
    expect(new Set(ranked).size).toBe(4);
  });

  test("concluído já revisado repousa; sem revisar, pede atenção", () => {
    expect(urgencyOf(session("a", done, { attention: false }))).toBe(0);
    expect(urgencyOf(session("b", done, { attention: true }))).toBeGreaterThan(
      0,
    );
  });
});

describe("lugar no layout", () => {
  test("acha o painel de cada sessão atravessando o split", () => {
    const split: PaneNode = {
      type: "split",
      id: "p0",
      split: "v",
      ratio: 0.5,
      first: leaf("p1", "s1"),
      second: leaf("p2", "s2"),
    } as PaneNode;
    const places = placesBySession(layout([workspace("w1", "w", [tab("t1", split)])]));

    expect(places.get("s1")?.paneId).toBe("p1");
    expect(places.get("s2")?.paneId).toBe("p2");
    expect(places.get("s1")?.tabId).toBe("t1");
  });

  test("o visualizador de subagente não vira uma segunda linha da mesma sessão", () => {
    const split: PaneNode = {
      type: "split",
      id: "p0",
      split: "v",
      ratio: 0.5,
      first: leaf("p1", "s1"),
      second: { type: "agentviewer", id: "p2", session_id: "s1" },
    } as PaneNode;

    const rows = buildRows(
      [session("s1", running)],
      layout([workspace("w1", "w", [tab("t1", split)])]),
    );

    expect(rows).toHaveLength(1);
    expect(rows[0].place.paneId).toBe("p1");
  });
});

describe("linhas do quadro", () => {
  test("sessão de shell não entra, nem quando está pedindo atenção", () => {
    // Sem hook não há gate; mostrá-la com a mesma cara diria que ela tem as
    // mesmas garantias das outras.
    const shell = session("s1", blocked, {
      kind: { type: "shell" } as SessionKind,
    });
    const rows = buildRows(
      [shell],
      layout([workspace("w1", "w", [tab("t1", leaf("p1", "s1"))])]),
    );

    expect(rows).toHaveLength(0);
  });

  test("agente sem painel no layout fica de fora — não haveria para onde saltar", () => {
    const rows = buildRows([session("s1", blocked)], layout([]));

    expect(rows).toHaveLength(0);
  });

  test("ordena por urgência e desempata por título", () => {
    const rows = buildRows(
      [
        session("zebra", running),
        session("alfa", running),
        session("bloqueada", blocked),
      ],
      layout([
        workspace("w1", "w", [
          tab("t1", leaf("p1", "zebra")),
          tab("t2", leaf("p2", "alfa")),
          tab("t3", leaf("p3", "bloqueada")),
        ]),
      ]),
    );

    expect(rows.map((r) => r.session.id)).toEqual([
      "bloqueada",
      "alfa",
      "zebra",
    ]);
  });
});

describe("rollup por workspace", () => {
  test("o grupo herda o estado do pior filho, não a contagem", () => {
    const rows = buildRows(
      [
        session("calma", running),
        session("presa", blocked),
        session("outra", running),
      ],
      layout([
        workspace("w1", "com bloqueio", [
          tab("t1", leaf("p1", "calma")),
          tab("t2", leaf("p2", "presa")),
        ]),
        workspace("w2", "sem bloqueio", [tab("t3", leaf("p3", "outra"))]),
      ]),
    );

    const groups = groupByWorkspace(rows);

    // O workspace com uma bloqueada entre duas vem antes do que só tem uma
    // rodando, embora tenha o mesmo tanto de agente calmo.
    expect(groups.map((g) => g.workspaceId)).toEqual(["w1", "w2"]);
    expect(groups[0].urgency).toBe(urgencyOf(session("presa", blocked)));
  });
});

describe("saltar para quem precisa", () => {
  const rows = () =>
    buildRows(
      [
        session("primeira", blocked),
        session("segunda", blocked),
        session("calma", running),
      ],
      layout([
        workspace("w1", "w", [
          tab("t1", leaf("p1", "primeira")),
          tab("t2", leaf("p2", "segunda")),
          tab("t3", leaf("p3", "calma")),
        ]),
      ]),
    );

  test("cicla em vez de prender na mais urgente", () => {
    // O caso que a implementação ingênua erra: com duas bloqueadas, "pular para
    // a mais urgente" devolveria sempre a primeira e a segunda nunca seria
    // alcançada.
    const all = rows();
    const first = nextAttention(all, null);
    expect(first?.session.id).toBe("primeira");

    const second = nextAttention(all, "primeira");
    expect(second?.session.id).toBe("segunda");

    expect(nextAttention(all, "segunda")?.session.id).toBe("primeira");
  });

  test("não oferece salto para quem está apenas rodando", () => {
    const all = rows();
    expect(all.filter(wantsAttention).map((r) => r.session.id)).toEqual([
      "primeira",
      "segunda",
    ]);
  });

  test("sem ninguém pedindo, não há para onde saltar", () => {
    const calma = buildRows(
      [session("calma", running)],
      layout([workspace("w1", "w", [tab("t1", leaf("p1", "calma"))])]),
    );
    expect(nextAttention(calma, null)).toBeNull();
  });
});
