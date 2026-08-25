import { describe, expect, test } from "bun:test";

import { mergeSessionUpdate, sameObserved } from "./sessionStatus";
import type { Session } from "./ipc";

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
