import { describe, expect, test } from "bun:test";

import type { AgentRow } from "./agentsBoard";
import type { ObservedAgent, Session, SessionKind, SessionStatus } from "./ipc";
import {
  DEFAULT_AGENT_ROWS,
  disambiguators,
  needsYou,
  rowHasValue,
  tokenValues,
  visibleRows,
  type AgentToken,
} from "./agentsSidebar";

const label = (key: string) => `«${key}»`;

const row = (
  status: SessionStatus,
  over: {
    observed?: ObservedAgent | null;
    hosting?: boolean;
    attention?: boolean;
    workspaceName?: string;
    id?: string;
    workspaceId?: string;
    order?: number;
  } = {},
): AgentRow => {
  const session = {
    id: over.id ?? "s1",
    kind: (over.observed
      ? { type: "shell" }
      : { type: "agent", runner: "claude_code" }) as SessionKind,
    title: "s1",
    repo_root: null,
    worktree: null,
    status,
    attention: over.attention ?? false,
    created_at: "",
    observed: over.observed ?? null,
  } as Session;
  return {
    session,
    place: {
      workspaceId: over.workspaceId ?? "w1",
      workspaceName: over.workspaceName ?? "rio-api",
      workspaceColor: null,
      tabId: "t1",
      paneId: "p1",
      order: over.order ?? 0,
    },
    observed: over.observed ?? null,
    hosting: over.hosting ?? false,
    visual: {
      dotClass: "bg-tyba-amber",
      textClass: "text-tyba-amber",
      labelKey: "sessionBlocked",
      rank: 3,
    },
    urgency: 31,
  };
};

const bloqueado: SessionStatus = {
  state: "awaiting_input",
  hint: "Aprovação pendente: git push",
  reason: "approval",
};
const rodando: SessionStatus = { state: "running" };
const concluiu: SessionStatus = { state: "idle", summary: "ajustei o parser" };

describe("valores dos tokens", () => {
  test("o detalhe de um gerenciado é o que ele espera", () => {
    const values = tokenValues(row(bloqueado), label);
    expect(values.detail).toBe("Aprovação pendente: git push");
    expect(values.no_gate).toBeNull();
    expect(values.agent).toBeNull();
  });

  test("observado traz o nome do agente e nenhum detalhe", () => {
    // A tela identifica quem é; ela não sabe no que ele travou, porque não há
    // gate. Inventar detalhe aqui seria afirmar o que não se sabe.
    const values = tokenValues(
      row(rodando, { observed: { agent: "claude-code", state: "running" } }),
      label,
    );
    expect(values.agent).toBe("claude-code");
    expect(values.detail).toBeNull();
    expect(values.no_gate).toBe(true);
  });

  test("concluído traz o resumo do turno", () => {
    expect(tokenValues(row(concluiu), label).detail).toBe("ajustei o parser");
  });

  test("observado hospedado pelo shim v2 (hosting) não leva o selo sem gate (tech-spec §7)", () => {
    const values = tokenValues(
      row(rodando, {
        observed: { agent: "claude-code", state: "running" },
        hosting: true,
      }),
      label,
    );
    expect(values.agent).toBe("claude-code");
    expect(values.no_gate).toBeNull();
  });
});

describe("linha vazia desaparece — a regra do herdr", () => {
  test("gerenciado rodando não gasta a segunda linha", () => {
    // O caso que a regra existe para pegar: `agent` é null (é gerenciado) e
    // `detail` é null (rodando não tem hint). Sem a regra, toda linha da seção
    // ocuparia dois níveis com metade em branco e a seção dobraria de altura
    // sem dizer nada a mais.
    const values = tokenValues(row(rodando), label);
    const linhas = visibleRows(DEFAULT_AGENT_ROWS, values);

    expect(linhas).toHaveLength(1);
    expect(linhas[0]).toEqual(["state_icon", "workspace", "no_gate"]);
  });

  test("a primeira linha nunca desaparece, porque o ícone é sempre um valor", () => {
    const values = tokenValues(row(rodando, { workspaceName: "" }), label);
    expect(rowHasValue(["state_icon", "workspace", "no_gate"], values)).toBe(
      true,
    );
  });

  test("linha de tokens todos vazios some", () => {
    const values = tokenValues(row(rodando), label);
    const sóVazios: AgentToken[] = ["agent", "detail", "no_gate"];
    expect(rowHasValue(sóVazios, values)).toBe(false);
  });

  test("observado usa as duas linhas", () => {
    const values = tokenValues(
      row(rodando, { observed: { agent: "opencode", state: null } }),
      label,
    );
    expect(visibleRows(DEFAULT_AGENT_ROWS, values)).toHaveLength(2);
  });
});

describe("a marca de precisa de você", () => {
  test("bloqueado leva a marca", () => {
    expect(needsYou(row(bloqueado))).toBe(true);
  });

  test("rodando não leva", () => {
    // Rodando é informação, não pedido. Marcar tudo que se mexe é o mesmo que
    // não marcar nada.
    const rodandoRow = row(rodando);
    rodandoRow.urgency = 10;
    expect(needsYou(rodandoRow)).toBe(false);
  });

  test("concluído sem revisão leva a marca", () => {
    const feito = row(concluiu, { attention: true });
    feito.urgency = 20;
    expect(needsYou(feito)).toBe(true);
  });

  test("a marca é independente da cor do estado", () => {
    // A cor diz o QUE ele está fazendo; a marca diz que ele parou por sua
    // causa. Um agente que falhou é vermelho e não espera ninguém.
    const falhou = row({ state: "failed", reason: "boom" });
    falhou.visual = { ...falhou.visual, textClass: "text-tyba-red" };
    falhou.urgency = 40;
    expect(needsYou(falhou)).toBe(true);
    expect(falhou.visual.textClass).toBe("text-tyba-red");
  });
});

describe("linhas que sairiam idênticas ganham desempate", () => {
  const dois = () => [
    row(rodando, {
      id: "a",
      order: 5,
      observed: { agent: "claude-code", state: null },
    }),
    row(rodando, {
      id: "b",
      order: 2,
      observed: { agent: "claude-code", state: null },
    }),
  ];

  test("duas sessões do mesmo workspace viram #1 e #2", () => {
    // O caso que apareceu na tela: mesmo workspace, mesmo agente, duas linhas
    // com exatamente o mesmo texto.
    const marcas = disambiguators(dois());
    expect(marcas.get("b")).toBe("#1");
    expect(marcas.get("a")).toBe("#2");
  });

  test("a ordem vem do layout, não da ordem da lista", () => {
    // Invertida na entrada, o resultado é o mesmo: quem tem `order` menor é
    // o #1. Sem isso o rótulo trocaria de linha a cada reordenação por
    // urgência, e um número que anda sozinho engana mais do que ajuda.
    const marcas = disambiguators(dois().reverse());
    expect(marcas.get("b")).toBe("#1");
    expect(marcas.get("a")).toBe("#2");
  });

  test("agente sozinho no workspace não ganha marca nenhuma", () => {
    const marcas = disambiguators([row(rodando, { id: "a" })]);
    expect(marcas.get("a")).toBeNull();
  });

  test("workspaces diferentes não disputam entre si", () => {
    const marcas = disambiguators([
      row(rodando, { id: "a", workspaceId: "w1" }),
      row(rodando, { id: "b", workspaceId: "w2" }),
    ]);
    expect(marcas.get("a")).toBeNull();
    expect(marcas.get("b")).toBeNull();
  });

  test("o desempate entra no texto do workspace", () => {
    const values = tokenValues(row(rodando), label, "#2");
    expect(values.workspace).toBe("rio-api #2");
  });

  test("sem desempate o nome fica limpo", () => {
    expect(tokenValues(row(rodando), label).workspace).toBe("rio-api");
  });
});
