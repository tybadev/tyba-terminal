import type { AgentRow } from "./agentsBoard";
import { rowShowsNoGate, wantsAttention } from "./agentsBoard";

/**
 * O que uma linha da seção de agentes pode mostrar.
 *
 * A linha é montada de **tokens**, e não de um layout cravado, seguindo o
 * `[ui.sidebar.agents] rows` do herdr. Hoje o default é fixo e não há
 * configuração exposta: o produto ainda não achou o layout certo, e abrir o
 * knob antes disso é empurrar a decisão para o usuário e chamar de
 * flexibilidade. Mas montar por token **agora** faz a customização, se um dia
 * houver evidência de que ela é precisa, virar expor o que já existe em vez de
 * reescrever a renderização.
 */
export type AgentToken =
  | "state_icon"
  | "state_text"
  | "workspace"
  | "agent"
  /** O que o agente espera, ou o resumo do que ele fez. O herdr não tem — ele
   *  não sabe no que o agente travou, porque não tem gate. */
  | "detail"
  /** Selo de "sem gate", para agente deduzido da tela. */
  | "no_gate";

/**
 * O layout padrão, em duas linhas.
 *
 * Espelha o default do herdr (`["state_icon","workspace","tab"]` /
 * `["agent"]`), com duas diferenças: não há `tab` porque no TYBA a coordenada
 * útil é o workspace — o salto leva ao painel, não à aba —, e há `detail`,
 * que é o que o gate permite dizer e o herdr não consegue.
 */
export const DEFAULT_AGENT_ROWS: AgentToken[][] = [
  ["state_icon", "workspace", "no_gate"],
  ["agent", "detail"],
];

export interface AgentTokenValues {
  state_icon: true;
  state_text: string;
  workspace: string;
  agent: string | null;
  detail: string | null;
  no_gate: true | null;
}

/**
 * O valor de cada token para esta linha. `null` significa "sem valor".
 *
 * `state_icon` e `no_gate` não têm texto — o valor deles é a presença. Ficam
 * aqui mesmo assim para que a regra de linha vazia (abaixo) os considere: uma
 * linha que só tem o selo ainda tem o que mostrar.
 */
export const tokenValues = (
  row: AgentRow,
  label: (key: string) => string,
  /** O desempate desta linha, de [`disambiguators`]. `null` quando ela já é
   *  única no seu workspace — que é o caso comum. */
  tie: string | null = null,
): AgentTokenValues => {
  const status = row.session.status;
  const detail = (() => {
    if (row.observed) return null;
    if (status.state === "awaiting_input" && status.hint) return status.hint;
    if (status.state === "failed") return status.reason;
    if (status.state === "idle") return status.summary;
    return null;
  })();
  return {
    state_icon: true,
    state_text: label(row.visual.labelKey),
    workspace: tie
      ? `${row.place.workspaceName} ${tie}`
      : row.place.workspaceName,
    agent: row.observed?.agent ?? null,
    detail,
    // Shim v2 (tech-spec §7): hospedado (hosting) já está dentro do gate,
    // então não mostra "sem gate" — ver `rowShowsNoGate`, fonte única desta
    // regra também no quadro e na fila.
    no_gate: rowShowsNoGate(row) ? true : null,
  };
};

/**
 * Regra do herdr, adotada literalmente: *"uma linha desaparece quando nenhum
 * dos seus tokens tem valor"*.
 *
 * É ela que impede a segunda linha de virar um espaço em branco em agente
 * gerenciado sem detalhe — sem isso, toda linha da lista ocuparia dois níveis
 * de altura, com metade vazia, e a seção dobraria de tamanho sem dizer nada.
 */
export const rowHasValue = (
  tokens: AgentToken[],
  values: AgentTokenValues,
): boolean => tokens.some((token) => values[token] != null);

/** As linhas que de fato serão pintadas, já sem as vazias. */
export const visibleRows = (
  layout: AgentToken[][],
  values: AgentTokenValues,
): AgentToken[][] => layout.filter((tokens) => rowHasValue(tokens, values));

/**
 * Esta linha leva a marca de "precisa de você"?
 *
 * Separado do visual de estado de propósito: a cor diz **o que** o agente está
 * fazendo, a marca diz **que ele parou por sua causa**. Um agente que falhou é
 * vermelho e não espera ninguém; um que aguarda aprovação é âmbar e espera.
 */
export const needsYou = (row: AgentRow): boolean => wantsAttention(row);

/**
 * O rótulo que separa linhas que sairiam idênticas.
 *
 * Duas sessões no mesmo workspace rodando o mesmo agente produzem duas linhas
 * com o mesmo texto, e uma lista em que dois itens são indistinguíveis não é
 * uma lista — é uma contagem. Foi o que apareceu na tela assim que o palpite
 * de tela voltou a chegar.
 *
 * **Só aparece onde há ambiguidade.** Marcar toda linha com um número seria
 * ruído em cima do caso comum, que é um agente por workspace; a mesma regra
 * do herdr para linha vazia, aplicada a rótulo.
 *
 * O número é a posição no LAYOUT, não na lista: a lista se reordena por
 * urgência a cada turno, e um rótulo que muda de lugar sozinho é pior que
 * rótulo nenhum — você decoraria "o #2" e ele viraria outro.
 *
 * Diz só o que se sabe: posição. Não promete "aba" nem "painel", porque as
 * duas sessões podem estar na mesma aba, em painéis divididos.
 */
export const disambiguators = (rows: AgentRow[]): Map<string, string | null> => {
  const porWorkspace = new Map<string, AgentRow[]>();
  for (const row of rows) {
    const key = row.place.workspaceId;
    const lista = porWorkspace.get(key);
    if (lista) lista.push(row);
    else porWorkspace.set(key, [row]);
  }
  const out = new Map<string, string | null>();
  for (const grupo of porWorkspace.values()) {
    if (grupo.length < 2) {
      for (const row of grupo) out.set(row.session.id, null);
      continue;
    }
    [...grupo]
      .sort((a, b) => a.place.order - b.place.order)
      .forEach((row, i) => out.set(row.session.id, `#${i + 1}`));
  }
  return out;
};
