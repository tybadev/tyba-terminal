import type { AgentRow } from "./agentsBoard";
import { wantsAttention } from "./agentsBoard";

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
    workspace: row.place.workspaceName,
    agent: row.observed?.agent ?? null,
    detail,
    no_gate: row.observed ? true : null,
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
