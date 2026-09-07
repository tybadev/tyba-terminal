import { describe, expect, test } from "bun:test";

import type {
  ApprovalRequest,
  LayoutState,
  ObservedAgent,
  ObservedState,
  PaneNode,
  Session,
  SessionKind,
  SessionStatus,
  Tab,
  Workspace,
} from "./ipc";
import {
  NO_SIGNAL,
  agentQueueVisibleApprovalIds,
  boardOrder,
  buildRows,
  groupByWorkspace,
  nextAttention,
  oldestApprovalBySession,
  placesBySession,
  rowShowsNoGate,
  urgencyOf,
  wantsAttention,
  agentForWorkspace,
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

/**
 * Sessão que o TYBA não lançou, com um agente deduzido da tela.
 *
 * O `status` é o do shell — `running`, como todo shell vivo — de propósito: é
 * ele que a implementação ingênua leria para pintar a linha, e o teste precisa
 * poder ver a diferença.
 */
const observedSession = (
  id: string,
  state: ObservedState | null,
  over: Partial<Session> = {},
): Session =>
  session(id, running, {
    kind: { type: "shell" } as SessionKind,
    observed: { agent: "claude", state },
    ...over,
  });

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
    ).managed;

    expect(rows).toHaveLength(1);
    expect(rows[0].place.paneId).toBe("p1");
  });
});

describe("linhas do quadro", () => {
  test("shell sem agente na tela não entra em nenhuma das seções", () => {
    // Um shell bloqueado é um shell esperando o usuário digitar, não um agente
    // parado. Sem sinal de agente na tela, ele não é assunto do quadro.
    const shell = session("s1", blocked, {
      kind: { type: "shell" } as SessionKind,
    });
    const sections = buildRows(
      [shell],
      layout([workspace("w1", "w", [tab("t1", leaf("p1", "s1"))])]),
    );

    expect(sections.managed).toHaveLength(0);
    expect(sections.observed).toHaveLength(0);
  });

  test("agente sem painel no layout fica de fora — não haveria para onde saltar", () => {
    const sections = buildRows([session("s1", blocked)], layout([]));

    expect(boardOrder(sections)).toHaveLength(0);
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
    ).managed;

    expect(rows.map((r) => r.session.id)).toEqual([
      "bloqueada",
      "alfa",
      "zebra",
    ]);
  });
});

describe("agente do workspace, para a barra lateral", () => {
  const mapa = (sessions: Session[]) =>
    new Map(sessions.map((s) => [s.id, s]));

  test("gerenciado ocioso vence observado bloqueado", () => {
    // A regra que dá sentido à separação: um palpite de tela nunca representa o
    // workspace no lugar de um agente que tem hook, gate e jaula — por mais
    // urgente que o palpite pareça.
    const gerenciado = session("gerenciado", done, { attention: true });
    const observado = session("observado", running, {
      kind: { type: "shell" } as SessionKind,
      observed: { agent: "claude-code", state: "awaiting_input" },
    });
    const w = workspace("w1", "w", [
      tab("t1", leaf("p1", "gerenciado")),
      tab("t2", leaf("p2", "observado")),
    ]);

    expect(agentForWorkspace(w, mapa([gerenciado, observado]))?.session.id).toBe(
      "gerenciado",
    );
  });

  test("sem gerenciado, o observado representa o workspace", () => {
    const observado = session("observado", running, {
      kind: { type: "shell" } as SessionKind,
      observed: { agent: "claude-code", state: "awaiting_input" },
    });
    const found = agentForWorkspace(
      workspace("w1", "w", [tab("t1", leaf("p1", "observado"))]),
      mapa([observado]),
    );

    expect(found?.session.id).toBe("observado");
    expect(found?.observed?.agent).toBe("claude-code");
  });

  test("o visual do observado não vem do status da sessão", () => {
    // Shell está SEMPRE `running`. Herdar o visual dele pintaria de azul,
    // "trabalhando", um agente parado esperando aprovação.
    const observado = session("observado", running, {
      kind: { type: "shell" } as SessionKind,
      observed: { agent: "claude-code", state: null },
    });
    const found = agentForWorkspace(
      workspace("w1", "w", [tab("t1", leaf("p1", "observado"))]),
      mapa([observado]),
    );

    expect(found?.visual.labelKey).toBe(NO_SIGNAL.labelKey);
  });

  test("shell sem agente nenhum não representa o workspace", () => {
    const shell = session("s1", running, {
      kind: { type: "shell" } as SessionKind,
    });
    expect(
      agentForWorkspace(
        workspace("w1", "w", [tab("t1", leaf("p1", "s1"))]),
        mapa([shell]),
      ),
    ).toBeNull();
  });

  test("entre dois observados vence o mais urgente", () => {
    const calmo = session("calmo", running, {
      kind: { type: "shell" } as SessionKind,
      observed: { agent: "opencode", state: null },
    });
    const preso = session("preso", running, {
      kind: { type: "shell" } as SessionKind,
      observed: { agent: "claude-code", state: "awaiting_input" },
    });
    const w = workspace("w1", "w", [
      tab("t1", leaf("p1", "calmo")),
      tab("t2", leaf("p2", "preso")),
    ]);

    expect(agentForWorkspace(w, mapa([calmo, preso]))?.session.id).toBe("preso");
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
    ).managed;

    const groups = groupByWorkspace(rows);

    // O workspace com uma bloqueada entre duas vem antes do que só tem uma
    // rodando, embora tenha o mesmo tanto de agente calmo.
    expect(groups.map((g) => g.workspaceId)).toEqual(["w1", "w2"]);
    expect(groups[0].urgency).toBe(urgencyOf(session("presa", blocked)));
  });
});

describe("saltar para quem precisa", () => {
  const rows = () =>
    boardOrder(
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
      ),
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
    const calma = boardOrder(
      buildRows(
        [session("calma", running)],
        layout([workspace("w1", "w", [tab("t1", leaf("p1", "calma"))])]),
      ),
    );
    expect(nextAttention(calma, null)).toBeNull();
  });
});

/** Um painel por sessão, tudo no mesmo workspace: o quadro sempre acha lugar. */
const board = (sessions: Session[]) =>
  buildRows(
    sessions,
    layout([
      workspace(
        "w1",
        "w",
        sessions.map((s, i) => tab(`t${i}`, leaf(`p${i}`, s.id))),
      ),
    ]),
  );

describe("seção dos observados", () => {
  test("agente na tela entra, mas na coleção separada", () => {
    const sections = board([observedSession("s1", "running")]);

    expect(sections.managed).toHaveLength(0);
    expect(sections.observed.map((r) => r.session.id)).toEqual(["s1"]);
    expect(sections.observed[0].observed?.agent).toBe("claude");
  });

  test("observado urgente não passa na frente de gerenciado calmo", () => {
    const sections = board([
      session("gerenciado", running),
      observedSession("observado", "awaiting_input"),
    ]);

    // O observado é mesmo o mais urgente dos dois — sem conferir isso o teste
    // passaria por acidente, e não por a fronteira entre as seções estar sendo
    // respeitada.
    expect(sections.observed[0].urgency).toBeGreaterThan(
      sections.managed[0].urgency,
    );
    // E ainda assim vem depois: um agente sem gate nunca empurra um gerenciado
    // para baixo, por mais aflito que a tela o faça parecer.
    expect(boardOrder(sections).map((r) => r.session.id)).toEqual([
      "gerenciado",
      "observado",
    ]);
  });

  test("a urgência ordena dentro da própria seção", () => {
    const sections = board([
      observedSession("rodando", "running"),
      observedSession("esperando", "awaiting_input"),
      observedSession("sem sinal", null),
    ]);

    expect(sections.observed.map((r) => r.session.id)).toEqual([
      "esperando",
      "rodando",
      "sem sinal",
    ]);
  });

  test("presença sem estado não recebe visual de estado", () => {
    const sections = board([
      observedSession("sem sinal", null),
      observedSession("rodando", "running"),
    ]);
    const semSinal = sections.observed.find(
      (r) => r.session.id === "sem sinal",
    )!;
    const rodando = sections.observed.find((r) => r.session.id === "rodando")!;

    expect(semSinal.visual.labelKey).toBe("agentsBoardNoSignal");
    expect(semSinal.visual.dotClass).not.toMatch(
      /bg-tyba-(amber|blue|green|red)/,
    );
    expect(semSinal.visual.dotClass).not.toMatch(/animate-pulse/);
    expect(semSinal.urgency).toBe(0);

    // O contraste que discrimina: estado deduzido de verdade ganha cor, e o
    // `null` não a herda do `status` do shell — que é `running` nos dois.
    expect(rodando.visual.dotClass).toMatch(/bg-tyba-blue/);
    expect(semSinal.visual.dotClass).not.toBe(rodando.visual.dotClass);
  });

  test("idle deduzido repousa: sem hook não há fim de turno a revisar", () => {
    // No gerenciado, idle só acende quando `attention` está de pé — e
    // `attention` é a marca que o hook levanta no fim do turno. Sessão sem hook
    // nunca a tem, então idle deduzido é repouso, não conclusão esperando
    // alguém.
    const [row] = board([observedSession("s1", "idle")]).observed;

    expect(row.urgency).toBe(0);
    expect(row.visual.dotClass).not.toMatch(/bg-tyba-green/);
  });

  test("sinal que este front não sabe ler vira presença sem estado", () => {
    // A união do TypeScript vale na compilação; o dado vem do core em tempo de
    // execução. Um `state` que o core acrescente sozinho — ou um `observed` sem
    // o campo — não pode estourar aqui: o `buildRows` roda dentro de `useMemo`,
    // onde a exceção não derruba a linha, derruba a renderização inteira.
    const futuro = observedSession("futuro", "compacting" as ObservedState);
    const semCampo = session("sem campo", running, {
      kind: { type: "shell" } as SessionKind,
      observed: { agent: "claude" } as ObservedAgent,
    });

    const rows = board([futuro, semCampo]).observed;

    // A linha aparece — presença é fato, e o selo "sem gate" vale igual.
    expect(rows.map((r) => r.session.id)).toEqual(["futuro", "sem campo"]);
    for (const row of rows) {
      expect(row.visual.labelKey).toBe("agentsBoardNoSignal");
      expect(row.visual.dotClass).not.toMatch(
        /bg-tyba-(amber|blue|green|red)/,
      );
      expect(row.urgency).toBe(0);
    }
  });

  test("gerenciado que por acidente traga `observed` conta uma vez só", () => {
    const dupla = session("s1", blocked, {
      observed: { agent: "claude", state: "idle" },
    });
    const sections = board([dupla]);

    expect(sections.managed.map((r) => r.session.id)).toEqual(["s1"]);
    expect(sections.observed).toHaveLength(0);
    expect(boardOrder(sections)).toHaveLength(1);
    // E sem o selo de "sem gate": a linha tem gate, e o selo sai do `observed`
    // da linha, nunca do da sessão.
    expect(sections.managed[0].observed).toBeNull();
    // O que ela mostra é o fato do hook, não o palpite da tela.
    expect(sections.managed[0].urgency).toBe(urgencyOf(dupla));
  });
});

describe("selo 'sem gate' respeita o hosting do shim v2 (tech-spec §7)", () => {
  test("observado sem hosting no mapa → linha sem gate (comportamento de sempre)", () => {
    const [row] = board([observedSession("s1", "running")]).observed;

    expect(row.hosting).toBe(false);
    expect(rowShowsNoGate(row)).toBe(true);
  });

  test("observado hospedado (hosting=true) → linha NÃO mostra sem gate", () => {
    const sections = buildRows(
      [observedSession("s1", "running")],
      layout([workspace("w1", "w", [tab("t1", leaf("p1", "s1"))])]),
      new Map([["s1", true]]),
    );
    const [row] = sections.observed;

    expect(row.hosting).toBe(true);
    expect(rowShowsNoGate(row)).toBe(false);
  });

  test("gerenciado nunca mostra sem gate, mesmo sem entrada no mapa de hosting", () => {
    const [row] = board([session("gerenciado", running)]).managed;

    expect(rowShowsNoGate(row)).toBe(false);
  });
});

describe("contagem de 'esperando por você'", () => {
  const waiting = (sessions: Session[]) =>
    boardOrder(board(sessions))
      .filter(wantsAttention)
      .map((r) => r.session.id);

  test("observado esperando conta — é o único que não tem outro canal", () => {
    // Decisão: entra. O badge do sidebar é o canal mais barato que existe — um
    // número que custa uma olhada quando erra —, enquanto a notificação tem
    // guardas próprias justamente porque interrompe: estado deduzido não é
    // afirmação forte o bastante para tirar o usuário do que ele está fazendo.
    // O agente sem gate é, porém, o único que não tem nenhum outro canal: não
    // há inbox, não há pedido de aprovação e não há hook que avise por ele.
    // Deixá-lo fora do badge seria escolher o silêncio exatamente para a linha
    // sobre a qual ninguém mais fala.
    expect(waiting([observedSession("observado", "awaiting_input")])).toEqual([
      "observado",
    ]);
  });

  test("contar não é promover: o gerenciado continua na frente na visita", () => {
    // O badge soma as duas seções, mas a ordem do ciclo não mistura: quem é
    // contado por último também é visitado por último.
    const sessions = [
      observedSession("observado", "awaiting_input"),
      session("gerenciado", blocked),
    ];

    expect(waiting(sessions)).toEqual(["gerenciado", "observado"]);
    expect(nextAttention(boardOrder(board(sessions)), null)?.session.id).toBe(
      "gerenciado",
    );
  });
});

const approval = (
  id: number,
  sessionId: string,
  requestedAtMs: number,
): ApprovalRequest => ({
  id,
  session_id: sessionId,
  command: "rm -rf build",
  cwd: null,
  risk: "green",
  context: null,
  requested_at_ms: requestedAtMs,
});

describe("oldestApprovalBySession", () => {
  test("sessão com dois pedidos pendentes: mapeia para o mais antigo", () => {
    const bySession = oldestApprovalBySession([
      approval(11, "s1", 200),
      approval(10, "s1", 100),
    ]);
    expect(bySession.get("s1")?.id).toBe(10);
  });

  test("sessões diferentes não se misturam", () => {
    const bySession = oldestApprovalBySession([
      approval(10, "s1", 100),
      approval(20, "s2", 50),
    ]);
    expect(bySession.get("s1")?.id).toBe(10);
    expect(bySession.get("s2")?.id).toBe(20);
  });
});

describe("agentQueueVisibleApprovalIds", () => {
  test("sessão com dois pedidos pendentes: só o mais antigo tem linha na fila — o mais novo fica sem ponto de ação ali", () => {
    const rows = boardOrder(board([session("s1", blocked)]));
    const ids = agentQueueVisibleApprovalIds(rows, [
      approval(11, "s1", 200),
      approval(10, "s1", 100),
    ]);
    expect(ids).toEqual(new Set([10]));
  });

  test("sessão sem linha na fila (sem lugar no layout) não esconde o próprio pedido", () => {
    const ids = agentQueueVisibleApprovalIds([], [approval(10, "s1", 100)]);
    expect(ids.size).toBe(0);
  });
});
