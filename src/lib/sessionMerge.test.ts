import { describe, expect, test } from "bun:test";

import { mergeSessionUpdate, sameObserved } from "./sessionStatus";
import type { Session, TurnDigest } from "./ipc";

const shell = (over: Partial<Session> = {}): Session =>
  ({
    id: "s1",
    kind: { type: "shell" },
    title: "rio-api",
    repo_root: null,
    worktree: null,
    status: { state: "running" },
    attention: false,
    created_at: "",
    observed: null,
    ...over,
  }) as Session;

describe("o palpite de tela sobrevive ao listener", () => {
  /**
   * O bug que o dono viu: `claude` cru rodando, a faixa âmbar na tela dizendo
   * que ele existe, e a seção de Agentes vazia.
   *
   * O shell fica `running` do primeiro ao último segundo e a atenção não se
   * mexe. O listener comparava só esses dois campos para decidir se havia
   * novidade — então o evento que trazia o agente era descartado como "nada
   * mudou", e o `observed` nunca chegava à lista.
   */
  test("agente aparecendo é novidade, mesmo com status e atenção iguais", () => {
    const antes = shell();
    const depois = shell({ observed: { agent: "claude-code", state: null } });

    const merged = mergeSessionUpdate(antes, depois);

    expect(merged).not.toBeNull();
    expect(merged?.observed).toEqual({ agent: "claude-code", state: null });
  });

  /**
   * A segunda metade do mesmo bug: mesmo quando o listener deixava passar (por
   * causa de outra mudança qualquer), a cópia era `{...c, status, attention}` —
   * `observed` ficava de fora e a sessão voltava sem agente.
   */
  test("mudança de status carrega o agente junto", () => {
    const antes = shell({ observed: { agent: "claude-code", state: null } });
    const depois = shell({
      status: { state: "idle", summary: "pronto" },
      attention: true,
      observed: { agent: "claude-code", state: "awaiting_input" },
    });

    const merged = mergeSessionUpdate(antes, depois);

    expect(merged?.observed).toEqual({
      agent: "claude-code",
      state: "awaiting_input",
    });
  });

  test("agente que sai também é novidade", () => {
    const antes = shell({ observed: { agent: "claude-code", state: null } });

    const merged = mergeSessionUpdate(antes, shell());

    expect(merged).not.toBeNull();
    expect(merged?.observed).toBeNull();
  });

  test("evento sem novidade nenhuma continua sendo descartado", () => {
    // O corte existe para não re-renderizar a lista a cada batida do PTY.
    const igual = shell({ observed: { agent: "claude-code", state: "running" } });
    expect(mergeSessionUpdate(igual, shell({ ...igual }))).toBeNull();
  });

  test("sessão encerrada não ressuscita por evento atrasado", () => {
    const morta = shell({ status: { state: "exited", code: 0 } });
    const atrasado = shell({ observed: { agent: "claude-code", state: null } });
    expect(mergeSessionUpdate(morta, atrasado)).toBeNull();
  });
});

describe("sameObserved", () => {
  test("ausência em ambos é igualdade", () => {
    expect(sameObserved(null, undefined)).toBe(true);
  });

  test("só o estado mudar já é diferença", () => {
    // É o que faz a linha trocar de cor quando o agente para de trabalhar.
    expect(
      sameObserved(
        { agent: "claude-code", state: "running" },
        { agent: "claude-code", state: "awaiting_input" },
      ),
    ).toBe(false);
  });

  test("agente diferente é diferença", () => {
    expect(
      sameObserved(
        { agent: "claude-code", state: null },
        { agent: "codex", state: null },
      ),
    ).toBe(false);
  });
});

describe("convite de retomar a conversa no shell restaurado", () => {
  test("o core desligar o convite chega ao pane", () => {
    const antes = shell({ shell_resume_offer: true });
    const depois = shell({ shell_resume_offer: false });

    const merged = mergeSessionUpdate(antes, depois);

    expect(merged?.shell_resume_offer).toBe(false);
  });

  test("convite igual dos dois lados não é novidade", () => {
    const antes = shell({ shell_resume_offer: true });
    expect(mergeSessionUpdate(antes, shell({ shell_resume_offer: true }))).toBeNull();
  });
});

describe("resumo do turno chega à pílula", () => {
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
    files_total: 2,
    insertions: 14,
    deletions: 3,
    commits: 0,
    cost_usd: null,
    tokens: null,
    ...over,
  });

  test("fim do turno é novidade, mesmo com status e atenção iguais", () => {
    const merged = mergeSessionUpdate(
      shell({ turn_open: true }),
      shell({ last_turn: digest(), turn_open: false }),
    );
    expect(merged?.last_turn?.files_total).toBe(2);
    expect(merged?.turn_open).toBe(false);
  });

  test("turno novo abrindo é novidade", () => {
    const last_turn = digest();
    const merged = mergeSessionUpdate(
      shell({ last_turn, turn_open: false }),
      shell({ last_turn, turn_open: true }),
    );
    expect(merged?.turn_open).toBe(true);
    expect(merged?.last_turn).toEqual(last_turn);
  });

  test("o custo que chega depois troca o resumo; o mesmo resumo não é novidade", () => {
    const antes = shell({ last_turn: digest(), turn_open: false });
    expect(mergeSessionUpdate(antes, shell({ last_turn: digest(), turn_open: false }))).toBeNull();
    const merged = mergeSessionUpdate(
      antes,
      shell({ last_turn: digest({ cost_usd: 0.42, tokens: 1200 }), turn_open: false }),
    );
    expect(merged?.last_turn?.cost_usd).toBe(0.42);
  });
});
