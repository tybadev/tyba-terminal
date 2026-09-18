// O que a tela decide sobre uma SSH Session integrada, sem tocar em React:
// se a sessão é integrada, o que a linha do pane explica, de onde vêm os
// chips, e — por onde a sessão fala — se o xterm.js ainda pode responder às
// consultas do terminal. Estado nenhum nasce aqui: tudo vem do core (eventos
// `ssh://integration`, `session://chips` e `session://transport`); esta camada
// só traduz.

import type {
  Integration,
  RemoteChips,
  RepoSnapshot,
  RepoStatus,
  SessionCommand,
  SessionKind,
  SessionTransport,
} from "./ipc";
import { programName } from "./commandLine";
import type { BranchChip } from "./repoSnapshots";

/**
 * Esta sessão fala a integração do TYBA?
 *
 * É o gate que substituiu `kind === "shell"` (regra 15): shell local sempre, e
 * SSH Session quando o core disse que ela nasceu integrada.
 *
 * `integration` ausente é "ainda não sei" e vale NÃO integrada — a resposta
 * chega em milissegundos por evento ou por `session_integration`, e assumir
 * "sim" nesse intervalo poria a linha do TYBA na frente de um terminal cru.
 */
export function isIntegratedSession(input: {
  kind: SessionKind | undefined;
  integration: Integration | null | undefined;
}): boolean {
  if (input.kind?.type === "shell") return true;
  return input.integration?.state === "integrated";
}

/** A linha que o pane mostra, já pronta para o `t()`. */
export interface IntegrationNotice {
  messageKey: string;
  params: Record<string, string>;
}

/**
 * A ressalva da sessão, em uma linha (regras 8, 12 e 13).
 *
 * São duas perguntas, nesta ordem: a sessão é comum — e por quê; e, sendo
 * integrada, ela sobrevive à queda do Cano. Uma sessão comum responde só a
 * primeira: falar de persistência antes de a sessão ser integrada responderia
 * a pergunta errada.
 *
 * `null` é o caso sem ressalva nenhuma: integrada e persistente, ou integrada
 * com persistência `unknown`.
 */
export function integrationNotice(
  integration: Integration | null | undefined,
): IntegrationNotice | null {
  if (integration?.state !== "plain") {
    // A sessão é integrada e o servidor não tem tmux (regra 13): a ressalva é
    // sobre persistência, não sobre integração. Só `ephemeral` produz linha —
    // `unknown` é o core sem resposta, e a tela não afirma o que ele não sabe.
    return integration?.persistence === "ephemeral"
      ? { messageKey: "sshIntegrationEphemeral", params: {} }
      : null;
  }
  if (integration.reason === "unsupported-shell") {
    // Sem o nome do shell a frase ficaria com um buraco no meio ("o shell  do
    // servidor"), então o motivo troca de frase em vez de trocar de valor.
    const shell = integration.detail?.trim();
    return shell
      ? { messageKey: "sshIntegrationUnsupportedShell", params: { shell } }
      : { messageKey: "sshIntegrationUnsupportedShellUnnamed", params: {} };
  }
  // `sshIntegrationPlain` cobre o motivo que esta versão da tela não conhece:
  // a sessão É comum, e dizer só isso é preferível a um pane mudo.
  return {
    messageKey: PLAIN_KEYS[integration.reason] ?? "sshIntegrationPlain",
    params: {},
  };
}

const PLAIN_KEYS: Record<string, string> = {
  "host-switch-off": "sshIntegrationOffSwitch",
  undetected: "sshIntegrationUndetected",
  "from-before": "sshIntegrationFromBefore",
};

/** Os chips da barra, de uma fonte só. */
export interface ToolbarChips {
  cwd: string | null;
  branch: BranchChip | null;
  /** Só a fonte local tem diffstat; numa sessão SSH é sempre `undefined`. */
  snapshot: RepoSnapshot | undefined;
  /** Arquivos mudados no SERVIDOR; `null` quando não há o que mostrar. */
  remoteChanged: number | null;
}

/**
 * De onde cada chip vem (regra 23).
 *
 * Numa SSH Session a resposta é "do servidor, ou de lugar nenhum" — nunca da
 * máquina local. O chip local ali não seria um valor aproximado: seria o
 * caminho de OUTRA máquina, com a branch de outro repositório.
 */
export function toolbarChips(input: {
  /** A sessão é SSH? É só isto que decide a fonte. */
  remote: boolean;
  chips: RemoteChips | null | undefined;
  local: ToolbarChips | Omit<ToolbarChips, "remoteChanged">;
}): ToolbarChips {
  if (input.remote) {
    const chips = input.chips;
    return {
      cwd: chips?.cwd ?? null,
      // `sessionId: null` deixa o chip como TEXTO, e é obrigatório: o seletor
      // de branch faz checkout pela sessão, e a sessão aqui é o `ssh` local —
      // o checkout cairia no repositório da máquina de cá.
      branch: chips?.git.branch
        ? { state: "known", label: chips.git.branch, sessionId: null }
        : null,
      snapshot: undefined,
      // Zero some, como o chip local já some com o repositório limpo — o chip
      // de diff é sinal de trabalho pendente, não um contador sempre presente.
      remoteChanged: chips?.git.changed ? chips.git.changed : null,
    };
  }
  const { cwd, branch, snapshot } = input.local;
  return { cwd, branch, snapshot, remoteChanged: null };
}

/** Os chips de git da linha da barra lateral, de uma fonte só. */
export interface SidebarChips {
  branch: string | null;
  /** O diffstat LOCAL; numa sessão remota é sempre `undefined`. */
  status: RepoStatus | undefined;
  /** Arquivos mudados no SERVIDOR; `null` quando não há o que mostrar. */
  remoteChanged: number | null;
}

/**
 * De onde a linha da barra lateral tira branch e diff (regra 23).
 *
 * Mesma regra de `toolbarChips`, e pelo mesmo motivo: quem decide a fonte é a
 * NATUREZA da sessão, não o momento. Armadilha medida: numa SSH Session o
 * processo `ssh` é local, e seu cwd é o diretório de onde o app subiu — o
 * snapshot local resolve e a linha exibe a branch e o diff do repositório de
 * cá até o `OSC 7` remoto chegar. Não é um chip atrasado: é o chip de outra
 * máquina. Ausência é honesta; valor local é mentira.
 */
export function sidebarChips(input: {
  /** A sessão daquela linha é SSH? É só isto que decide a fonte. */
  remote: boolean;
  /**
   * A preferência "status do git na sidebar". Ela fala da LINHA, não da fonte:
   * desligada, some o chip de diff dos dois lados — o do servidor também.
   */
  showStatus: boolean;
  chips: RemoteChips | null | undefined;
  local: {
    branch: string | null | undefined;
    status: RepoStatus | null | undefined;
  };
}): SidebarChips {
  if (input.remote) {
    const changed = input.chips?.git.changed;
    return {
      branch: input.chips?.git.branch ?? null,
      // O servidor responde quantos arquivos mudaram, e só — não há linhas
      // somadas nem removidas para o `DiffStat`, e por isso a contagem remota
      // viaja em campo próprio. Zero some, como o chip local já some com o
      // repositório limpo: o chip de diff é sinal de trabalho pendente.
      status: undefined,
      remoteChanged: input.showStatus && changed ? changed : null,
    };
  }
  return {
    branch: input.local.branch ?? null,
    status: input.showStatus ? (input.local.status ?? undefined) : undefined,
    remoteChanged: null,
  };
}

/**
 * A faixa de agente sem jaula da sessão remota (regra 26).
 *
 * Quem diz que o comando é um agente é o core (`remote_agent_without_jail`) —
 * aqui só se decide se a faixa está na tela agora. `confirmed` é o comando que
 * o core confirmou: comparar com o que está rodando impede que a resposta de
 * um comando pinte a faixa sobre o seguinte.
 */
export function remoteAgentNotice(input: {
  command: SessionCommand | undefined;
  confirmed: string | null | undefined;
}): { binary: string } | null {
  const running = input.command;
  if (!running?.running || !running.command) return null;
  if (input.confirmed !== running.command) return null;
  return { binary: programName(running.command) ?? running.command };
}

/** A faixa do topo do pane numa sessão remota: no máximo uma por vez. */
export interface PaneNotice extends IntegrationNotice {
  tone: "amber" | "cyan";
}

/**
 * Qual faixa o pane mostra.
 *
 * Uma só, porque as duas ocupam o mesmo lugar do pane. O agente ganha: a linha
 * de integração descreve uma condição permanente da sessão, e o agente sem
 * jaula é o que está acontecendo AGORA.
 */
export function paneNotice(input: {
  integration: Integration | null | undefined;
  agent: { binary: string } | null;
}): PaneNotice | null {
  if (input.agent) {
    return {
      messageKey: "sshRemoteAgentNotice",
      params: { binary: input.agent.binary },
      tone: "amber",
    };
  }
  const notice = integrationNotice(input.integration);
  return notice ? { ...notice, tone: "cyan" } : null;
}

/** O transporte de uma sessão antes de o core dizer qualquer coisa. */
export const INITIAL_TRANSPORT: SessionTransport = "raw";

/**
 * O transporte depois de um anúncio do core (`session://transport/<id>`).
 *
 * Segue o último anúncio, inclusive de volta para `raw`: um respawn no mesmo id
 * (reconexão do Cano) reanuncia o estado inicial, e a sessão pode renascer
 * comum — integração desligada, host sem tmux. Uma máquina só-de-ida pareceria
 * mais segura e calaria o xterm para sempre nesse caso.
 *
 * Valor que esta versão não conhece mantém o estado: o front não adivinha
 * transporte (princípio #1), e "não entendi" nunca vira "responda".
 */
export function nextTransport(
  current: SessionTransport,
  announced: string,
): SessionTransport {
  if (announced === "tmux_control" || announced === "raw") return announced;
  return current;
}

/** Um identificador de sequência como o xterm.js registra (`IFunctionIdentifier`). */
export interface SequenceId {
  prefix?: string;
  intermediates?: string;
  final: string;
}

/** `true` quando a sequência, com ESTES parâmetros, gera resposta no xterm.js. */
type IsReport = (params: readonly (number | number[])[]) => boolean;

const ALWAYS: IsReport = () => true;

/** O primeiro parâmetro, já sem os subparâmetros (`38:2:...`). */
function firstParam(params: readonly (number | number[])[]): number {
  const p = params[0];
  return Array.isArray(p) ? (p[0] ?? 0) : (p ?? 0);
}

/** O segundo parâmetro; `0` quando a sequência não o traz, como no xterm.js. */
function secondParam(params: readonly (number | number[])[]): number {
  const p = params[1];
  if (p === undefined) return 0;
  return Array.isArray(p) ? (p[0] ?? 0) : p;
}

export interface QuerySequence {
  /** Qual `register*Handler` do xterm.js assina esta sequência. */
  kind: "csi" | "dcs";
  id: SequenceId;
  isReport: IsReport;
}

/**
 * Tudo que o xterm.js 5.5.0 responde por conta própria — levantado no
 * `common/InputHandler.ts` do sourcemap do pacote, não de memória.
 *
 * É a MESMA lista que o pane assina e que `silenceReply` consulta: separar as
 * duas deixaria uma sequência assinada sem regra, ou uma regra que ninguém
 * chama.
 */
export const QUERY_SEQUENCES: readonly QuerySequence[] = [
  // DA1 — `sendDeviceAttributesPrimary`.
  { kind: "csi", id: { final: "c" }, isReport: ALWAYS },
  // DA2 — `sendDeviceAttributesSecondary`.
  { kind: "csi", id: { prefix: ">", final: "c" }, isReport: ALWAYS },
  // DA3. O xterm.js 5.5.0 não a registra, então hoje isto não muda nada — está
  // na lista para que passar a responder não vire lixo no prompt do servidor.
  { kind: "csi", id: { prefix: "=", final: "c" }, isReport: ALWAYS },
  // DSR — `deviceStatus`: só 5 (status) e 6 (CPR) respondem.
  {
    kind: "csi",
    id: { final: "n" },
    isReport: (params) => firstParam(params) === 5 || firstParam(params) === 6,
  },
  // DECXCPR — `deviceStatusPrivate`: o xterm.js reconhece 6, 15, 25, 26 e 53,
  // e só o 6 tem resposta.
  {
    kind: "csi",
    id: { prefix: "?", final: "n" },
    isReport: (params) => firstParam(params) === 6,
  },
  // DECRQM — `requestMode`: responde a QUALQUER modo, inclusive com
  // `NOT_RECOGNIZED`, nos dois sabores (ANSI e privado).
  { kind: "csi", id: { intermediates: "$", final: "p" }, isReport: ALWAYS },
  {
    kind: "csi",
    id: { prefix: "?", intermediates: "$", final: "p" },
    isReport: ALWAYS,
  },
  // XTWINOPS — `windowOptions`: 14 (área em pixels, exceto o `14;2` que pede a
  // janela), 16 (célula em pixels) e 18 (área em células) respondem; 22 e 23
  // mexem na pilha de título e não respondem nada.
  //
  // Armadilha ao testar: com `windowOptions` no padrão (tudo desligado, que é
  // como o TYBA constrói o Terminal) o xterm.js engole o `t` ANTES de chamar
  // handler de terceiro — e também não responde. Esta regra vale para o dia em
  // que alguma dessas opções for ligada.
  {
    kind: "csi",
    id: { final: "t" },
    isReport: (params) => {
      const op = firstParam(params);
      if (op === 14) return secondParam(params) !== 2;
      return op === 16 || op === 18;
    },
  },
  // DECRQSS — `requestStatusString`: responde a tudo, nem que seja o `P0$r` de
  // "não sei". É a única DCS da lista.
  {
    kind: "dcs",
    id: { intermediates: "$", final: "q" },
    isReport: ALWAYS,
  },
];

function sequenceKey(kind: "csi" | "dcs", id: SequenceId): string {
  return `${kind}|${id.prefix ?? ""}|${id.intermediates ?? ""}|${id.final}`;
}

const REPORTS = new Map<string, IsReport>(
  QUERY_SEQUENCES.map((q) => [sequenceKey(q.kind, q.id), q.isReport]),
);

/**
 * O pane responde a esta consulta, ou fica calado?
 *
 * Num transporte de controle, calar é o certo: o tmux remoto JÁ respondeu à
 * consulta (`input_reply` escreve no pane) e ainda encaminhou os bytes crus da
 * consulta ao cliente de controle. A resposta do xterm.js seria a segunda, e
 * daqui só pode voltar como `send-keys` — ou seja, digitação no pane: foi assim
 * que `1;2c0;276;0c` apareceu no prompt do servidor. Um cliente de controle não
 * tem tty e, por desenho do protocolo, não responde consulta nenhuma.
 *
 * `false` devolve a sequência ao handler padrão do xterm.js — é o que mantém a
 * sessão `raw` idêntica ao que sempre foi, e o que preserva o tratamento das
 * sequências de final ambíguo que não são relatório.
 */
export function silenceReply(input: {
  transport: SessionTransport;
  id: SequenceId;
  params: readonly (number | number[])[];
  /** CSI é o padrão porque é o caso de quase toda a lista. */
  kind?: "csi" | "dcs";
}): boolean {
  if (input.transport !== "tmux_control") return false;
  const isReport = REPORTS.get(sequenceKey(input.kind ?? "csi", input.id));
  return isReport ? isReport(input.params) : false;
}

/** As OSC que o xterm.js responde quando o payload traz `?` (relato de cor). */
export const COLOR_QUERY_OSC: readonly number[] = [4, 10, 11, 12];

/**
 * A OSC de cor é consulta — e portanto calada no transporte de controle?
 *
 * A OSC 4 vem em PARES `índice;cor`, e só o slot da cor pode ser `?`; um `?` no
 * lugar do índice o xterm.js descarta sem responder. As 10/11/12 são uma lista
 * de cores, e qualquer `?` nela é um relato.
 *
 * Armadilha: um payload que mistura pintar e perguntar (`4;1;#ff0000;2;?`) é
 * calado INTEIRO — o handler é tudo ou nada, então a cor do meio não é pintada.
 * Perder uma cor é cosmético; devolver a resposta é digitação no pane.
 */
export function silenceOscReply(input: {
  transport: SessionTransport;
  ident: number;
  data: string;
}): boolean {
  if (input.transport !== "tmux_control") return false;
  if (!COLOR_QUERY_OSC.includes(input.ident)) return false;
  const slots = input.data.split(";");
  if (input.ident === 4) {
    for (let i = 1; i < slots.length; i += 2) {
      if (slots[i] === "?") return true;
    }
    return false;
  }
  return slots.includes("?");
}
