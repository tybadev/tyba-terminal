import { describe, expect, test } from "bun:test";

import { sessionBlocksData } from "./sessionBlocks";
import { callLatest } from "./stableProps";

/**
 * A comparação que o `React.memo` faz: `Object.is` campo a campo sobre as
 * props. Mesma técnica de `sessionBlocks.test.ts` — o defeito aqui é o mesmo,
 * só que na metade das props que aquele teste não cobre: os handlers.
 */
function shallowEqual(a: object, b: object): boolean {
  const left = Object.keys(a) as Array<keyof typeof a>;
  const right = Object.keys(b);
  if (left.length !== right.length) return false;
  return left.every((key) => Object.is(a[key], b[key]));
}

/**
 * O que o `useCallback` faz, sem React: guarda a função e devolve a MESMA
 * enquanto nenhuma dependência mudar por `Object.is`.
 *
 * Existe para que o vermelho seja o comportamento real do hook, e não uma
 * descrição dele. O teste "devolve a mesma função quando nada muda" afere o
 * instrumento antes de os outros o usarem.
 */
function useCallbackSim<T>() {
  let deps: readonly unknown[] | null = null;
  let held: T | null = null;
  return (fn: T, next: readonly unknown[]): T => {
    const same =
      deps !== null &&
      deps.length === next.length &&
      next.every((dep, i) => Object.is(dep, deps![i]));
    if (!same) {
      deps = next;
      held = fn;
    }
    return held as T;
  };
}

/** O estado do `App` que o `injectIntoActive` lê. */
interface AppState {
  activeId: string;
  /** Vira a cada começo e fim de comando. */
  ownsCommandLine: boolean;
}

const AT_PROMPT: AppState = { activeId: "s1", ownsCommandLine: true };
const RUNNING: AppState = { activeId: "s1", ownsCommandLine: false };

// Os outros handlers do painel já são estáveis: `focusPane` e `clearPick` são
// `useCallback` sem dependência viva, e `onPick` sai do `handlerCache`.
const onFocusPane = () => Promise.resolve();
const onPick = () => {};
const onClearPick = () => {};
const onHeaderPx = () => {};

/**
 * As props do `SessionBlocks` de um painel ESPECTADOR — outra sessão, que não é
 * a ativa e não tem comando nenhum rodando.
 *
 * É esse o painel que prova o defeito: nada no estado dele muda quando um
 * comando começa na sessão vizinha, então todo campo de dado sai idêntico e a
 * única prop capaz de derrubar a comparação é o handler.
 */
function bystanderProps(onInject: (text: string) => void) {
  return {
    ...sessionBlocksData({
      session: { id: "s2", created_at: "2026-08-21T10:00:00Z" },
      pane: { pane: "p2", x: 50, y: 0, w: 50, h: 100 },
      blocks: undefined,
      live: false,
      used: undefined,
      headerPx: undefined,
      fontSizePx: 13,
      lineHeightPx: 21.5,
      cellWidthPx: 8,
      cwd: undefined,
      active: false,
      command: undefined,
      marked: null,
      copyCombo: "⌘C",
    }),
    onInject,
    onFocusPane,
    onPick,
    onClearPick,
    onHeaderPx,
  };
}

describe("useCallbackSim", () => {
  test("devolve a MESMA função quando nada muda", () => {
    // Afere o instrumento: sem isto, um verde adiante poderia ser só uma
    // simulação que nunca troca de função.
    const useCb = useCallbackSim<() => void>();
    const render = (state: AppState) =>
      useCb(() => {}, [state.activeId, state.ownsCommandLine]);
    expect(render(AT_PROMPT)).toBe(render(AT_PROMPT));
  });
});

describe("onInject como useCallback com dependências", () => {
  /** Era assim que o `App` montava o handler. */
  const withDeps = () => {
    const useCb = useCallbackSim<(text: string) => void>();
    return (state: AppState) =>
      useCb(() => {}, [state.activeId, state.ownsCommandLine]);
  };

  test("a fronteira de comando invalida o memo do painel espectador", () => {
    // O vermelho. `ownsCommandLine` vira quando um comando começa na sessão
    // ATIVA; o painel de baixo continua parado e mesmo assim re-renderiza,
    // porque a prop dele nasceu de novo.
    const render = withDeps();
    const before = bystanderProps(render(AT_PROMPT));
    const after = bystanderProps(render(RUNNING));
    expect(shallowEqual(before, after)).toBe(false);
    // E é o handler, não um campo de dado: todo o resto saiu idêntico.
    const { onInject: _a, ...dataBefore } = before;
    const { onInject: _b, ...dataAfter } = after;
    expect(shallowEqual(dataBefore, dataAfter)).toBe(true);
  });

  test("trocar o foco de painel invalida de novo", () => {
    const render = withDeps();
    expect(
      shallowEqual(
        bystanderProps(render(AT_PROMPT)),
        bystanderProps(render({ activeId: "s2", ownsCommandLine: true })),
      ),
    ).toBe(false);
  });
});

describe("callLatest", () => {
  test("a identidade sobrevive à fronteira de comando", () => {
    // O verde, pelo MESMO simulador do vermelho — a única diferença entre os
    // dois é a lista de dependências. É o que o `useStableCallback` monta: o
    // ref recebe a closure nova a cada render, e a função exposta sai de um
    // `useMemo` com lista vazia.
    const useMemoSim = useCallbackSim<(text: string) => void>();
    const ref = { current: (_text: string) => {} };
    const render = (state: AppState) => {
      ref.current = (_text: string) => void state;
      return useMemoSim(callLatest(ref), []);
    };
    expect(
      shallowEqual(
        bystanderProps(render(AT_PROMPT)),
        bystanderProps(render(RUNNING)),
      ),
    ).toBe(true);
  });

  test("chama a closure do último render, não a do primeiro", () => {
    // O risco que a identidade fixa introduz, e a razão de esta função existir
    // separada do hook: congelar a identidade e congelar junto o `activeId`
    // faria o texto entrar no painel errado.
    const ref = { current: (text: string) => `s1:${text}` };
    const stable = callLatest(ref);
    expect(stable("ls")).toBe("s1:ls");
    ref.current = (text: string) => `s2:${text}`;
    expect(stable("ls")).toBe("s2:ls");
  });

  test("repassa todos os argumentos e o retorno", () => {
    const ref = { current: (a: number, b: number) => a + b };
    expect(callLatest(ref)(2, 3)).toBe(5);
  });
});
