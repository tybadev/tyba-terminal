import type { SessionCommand, SessionKind } from "./ipc";

export const PROMPT_MODE_PREF_KEY = "pref.promptMode";

/**
 * A linha do TYBA vem LIGADA; só desliga quem pediu.
 *
 * Ela nasceu opt-in, e o efeito era que a instalação nova entregava um
 * terminal cru: para ver o TYBA que o produto promete — blocos, histórico,
 * estado de comando —, era preciso descobrir uma caixa de configuração. O
 * padrão é o produto.
 *
 * Ausência de preferência é ligada, e não desligada, porque quem nunca abriu
 * as configurações não escolheu nada — e a escolha por omissão tem de ser a
 * que serve a quem acabou de instalar. `"off"` é a única resposta que desliga,
 * o que também mantém a semântica de quem já gravou a preferência: quem tinha
 * ligado continua ligado, e quem tinha desligado continua desligado.
 *
 * Mesma forma que `HISTORY_PREF_KEY` já usava, e vive aqui — e não nos dois
 * pontos de leitura — porque `App.tsx` e `ShellSettings.tsx` liam a mesma
 * chave com o próprio default cada um, que é como os dois divergem.
 */
export function promptModeEnabled(value: string | null | undefined): boolean {
  return value !== "off";
}

/**
 * Quem é dono do teclado neste instante.
 *
 * `terminal` não é fallback de UX, é correção: `ssh`, `read`, `psql` e o prompt
 * de senha do `sudo` leem stdin DURANTE o comando. Se a linha do TYBA engolir
 * essas teclas, o usuário digita a senha num campo que não vai a lugar nenhum.
 */
export type KeyboardOwner = "terminal" | "tybaLine";

export interface OwnerInput {
  /** Preferência ligada e o shell iniciado em modo prompt do TYBA. */
  promptMode: boolean;
  kind: SessionKind | undefined;
  /** `term.buffer.active.type === "alternate"` — vim, htop, less. */
  altScreen: boolean;
  /** Entre `133;C` e `133;D`. */
  command: SessionCommand | undefined;
  /** Sem `133;A` não há como saber que o shell está no prompt. */
  integrated: boolean;
}

/**
 * Por que a linha não é editável agora — ela nunca desaparece.
 *
 * Sumir e voltar a cada comando redimensionava o terminal duas vezes por
 * execução, e o `vim` reabria com outra altura.
 */
export type LineState =
  | "own"
  | "waiting"
  | "running"
  | "continuation"
  | "app"
  | "off";

/** O shell respondeu que NÃO está em modo prompt, mas o usuário quer estar. */
export function isOff(input: OwnerInput & { reported: boolean | undefined }) {
  return input.reported === false;
}

export function lineState(
  input: OwnerInput & { reported?: boolean | undefined },
): LineState {
  if (keyboardOwner(input) === "tybaLine") return "own";
  // Desligado de propósito (ou por engano num ⌘⇧L a mais): a linha continua na
  // tela dizendo isso. Ela sumir sem explicação foi o que fez o modo clássico
  // parecer defeito.
  if (input.reported === false) return "off";
  if (input.altScreen) return "app";
  if (input.command?.running) return "running";
  // Continuação vem DEPOIS de `running` na ordem porque os dois nunca são
  // verdadeiros juntos — a ordem aqui é legibilidade, não precedência.
  if (input.command?.continuation) return "continuation";
  return "waiting";
}

/**
 * A caixa de digitar existe no DOM neste estado?
 *
 * > [!warning] `app` e `off` a trocam pela faixa de uma linha. `waiting`,
 * > `running` e `continuation` mantêm a MESMA textarea montada — desabilitada,
 * > com o rascunho dentro. Quem confundir "não é minha" com "não está na tela"
 * > mexe numa caixa que o usuário está olhando: zerar a altura medida ali
 * > esconde o texto que ele escreveu, e a altura não volta sozinha porque
 * > quem a escreve é a medição do conteúdo, não o CSS.
 *
 * `off` entrou aqui em 22/08: no modo clássico a caixa ficava montada e
 * desabilitada, ~36px de input morto no rodapé com o prompt de verdade no
 * terminal logo acima. Não é exceção à regra de que a linha nunca some — é a
 * mesma saída que `app` já usava. A regra existe contra resize **por comando**,
 * dezenas de vezes por sessão; trocar de modo é deliberado e raro, e custa um
 * resize só.
 */
export function boxIsMounted(state: LineState): boolean {
  return state !== "app" && state !== "off";
}

/**
 * A caixa aceita digitação neste estado?
 *
 * > [!warning] `waiting` é editável, e isso é a correção — não uma folga.
 * > Ele é o intervalo entre a sessão abrir e o shell reportar o primeiro
 * > prompt: 1,4 s no `.zshrc` do dono, medido em pty real. Desabilitada ali, a
 * > caixa não recebe tecla NENHUMA — textarea desabilitada não dispara
 * > `keydown` —, então o comando digitado no primeiro segundo de cada sessão
 * > não aparece em lugar nenhum e o Enter não faz nada. É o "digitei um
 * > comando, apertei Enter e não aconteceu nada".
 * >
 * > Editável, o rascunho fica na caixa e o Enter vira uma submissão que o core
 * > segura até o shell abrir a linha dele (ver `LineEditorGate`), em vez de
 * > escrever num tty canônico que ecoaria a injeção crua na tela.
 *
 * Os outros continuam fechados, e por motivos que não mudaram: em `running` e
 * `continuation` quem lê o teclado é o comando, em `app` é o programa de tela
 * cheia, e `off` é o shell tendo respondido que NÃO está em modo prompt — ali a
 * linha do TYBA não teria para onde enviar.
 */
export function boxAcceptsTyping(state: LineState): boolean {
  return state === "own" || state === "waiting";
}

export function keyboardOwner({
  promptMode,
  kind,
  altScreen,
  command,
  integrated,
}: OwnerInput): KeyboardOwner {
  if (!promptMode || !integrated) return "terminal";
  if (kind?.type !== "shell") return "terminal";
  if (altScreen) return "terminal";
  if (command?.running) return "terminal";
  // O shell está no meio de um comando multi-linha, esperando o resto.
  //
  // Sem esta linha o front só via `running: false` — o `PS2` não emite OSC
  // nenhum — e devolvia o teclado para a caixa do TYBA, que oferecia começar
  // um comando novo. O que o usuário digitasse ali viraria uma submissão
  // separada em vez do corpo do `for` que ele estava escrevendo.
  if (command?.continuation) return "terminal";
  return "tybaLine";
}

/** As quatro setas, pelo `key` do evento de teclado. */
const ARROW_KEYS = new Set([
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
]);

export function isArrowKey(key: string): boolean {
  return ARROW_KEYS.has(key);
}

export interface ArrowInput {
  /** Entre `133;C` e `133;D`. Fora disso a linha do TYBA já é dona do teclado. */
  running: boolean;
  /**
   * `ECHO` do termios: o tty entrega LINHAS, não teclas.
   *
   * Ligado, o driver só devolve a linha ao dar Enter e trata apenas
   * backspace/kill — a seta entra como byte literal que nenhum leitor de linha
   * interpreta, e ainda é ecoada de volta.
   */
  lineEcho: boolean;
  /** vim, htop, less: a tela é do programa e as setas também. */
  altScreen: boolean;
}

/**
 * A seta morre aqui em vez de ir para o PTY?
 *
 * O caso: com um comando rodando, apertar seta escrevia `^[[A` na saída — e a
 * saída vira bloco gravado no SQLite e no markdown do copiar. Some da tela ao
 * limpar, não do disco.
 *
 * Só quando o tty está em modo linha, e essa condição é a causa, não um proxy
 * dela: é exatamente nesse estado que a seta não serve a ninguém e é ecoada.
 * Em raw, quem lê tecla a tecla precisa dela — o menu do `npm create`, que é
 * canônico ao perguntar `Ok to proceed? (y)` e vira raw ao abrir a lista, no
 * mesmo comando.
 *
 * Não vale para o resto do teclado: o `y` daquele prompt também é canônico com
 * eco, e engoli-lo impediria responder. Ver {@link KeyboardOwner}.
 */
export function swallowsArrow({
  running,
  lineEcho,
  altScreen,
}: ArrowInput): boolean {
  if (!running || altScreen) return false;
  return lineEcho;
}

/**
 * Teclas que a linha do TYBA nunca consome: são sinais para o processo, não
 * texto. Ctrl+C também limpa a caixa — o usuário espera perder o rascunho.
 */
const CONTROL_KEYS: Record<string, string> = {
  c: "\x03",
  d: "\x04",
  z: "\x1a",
  "\\": "\x1c",
};

export interface ControlChord {
  key: string;
  ctrl: boolean;
  meta: boolean;
  alt: boolean;
}

export function controlBytes(chord: ControlChord): string | null {
  if (!chord.ctrl || chord.meta || chord.alt) return null;
  return CONTROL_KEYS[chord.key.toLowerCase()] ?? null;
}

export function clearsDraft(chord: ControlChord): boolean {
  return controlBytes(chord) === "\x03";
}

export const SUGGEST_DEBOUNCE_MS = 70;

export interface PathToken {
  /** Índice onde o token começa, para trocá-lo sem tocar no resto da linha. */
  start: number;
  value: string;
}

const PATHISH = /^(\.{1,2}\/|~|\/)/;

/**
 * O token sob o cursor e o que vem antes dele na linha.
 *
 * O prefixo é o que dá contexto: `git ` completa subcomando, `cargo test `
 * completa flag. Sem ele a sugestão seria "qualquer palavra que já digitei",
 * que é trabalho do histórico.
 */
export interface LineToken {
  start: number;
  value: string;
  prefix: string;
}

export function lineToken(text: string, caret: number): LineToken | null {
  const before = text.slice(0, caret);
  const match = /[^\s]*$/.exec(before);
  if (!match) return null;
  const value = match[0];
  const start = before.length - value.length;
  const prefix = before.slice(0, start);
  if (!prefix.trim()) return null;
  return { start, value, prefix };
}

/**
 * O token que deve ser completado como caminho, ou `null`.
 *
 * A primeira palavra da linha é posição de COMANDO, não de arquivo: num
 * diretório com `teste/`, completar `te` para `teste/` transformaria o começo de
 * `test` numa pasta. Só vale como caminho se for argumento, ou se o próprio
 * token já disser que é caminho (`./`, `../`, `~`, `/`, ou contém barra).
 */
export function pathToken(text: string, caret: number): PathToken | null {
  const before = text.slice(0, caret);
  const match = /[^\s]*$/.exec(before);
  if (!match) return null;
  const value = match[0];
  if (!value) return null;
  const start = before.length - value.length;
  const isFirstWord = before.slice(0, start).trim().length === 0;
  if (isFirstWord && !PATHISH.test(value) && !value.includes("/")) return null;
  return { start, value };
}

/** Troca só o token do caminho, preservando o resto da linha. */
export function replaceToken(
  text: string,
  token: PathToken,
  completion: string,
): { text: string; caret: number } {
  const next =
    text.slice(0, token.start) +
    completion +
    text.slice(token.start + token.value.length);
  return { text: next, caret: token.start + completion.length };
}

export interface Suggestion {
  command: string;
}

/**
 * O resto do comando que aparece em cinza. Só a primeira sugestão que
 * **começa** com o que já foi digitado serve — ghost text que não é prefixo
 * mentiria sobre o que o `→` vai completar.
 */
export function ghostFor(text: string, hits: Suggestion[]): string {
  if (!text.trim()) return "";
  const hit = hits.find(
    (candidate) =>
      candidate.command.length > text.length &&
      candidate.command.startsWith(text),
  );
  return hit ? hit.command.slice(text.length) : "";
}

/**
 * As flags de um wrapper, separadas pelo que interessa aqui: consumir ou não o
 * próximo token.
 *
 * `value` é a lista que evita o defeito — `sudo -u app git push` tem `app` como
 * OPERANDO do `-u`, não como programa. `bare` existe para o outro lado: sem
 * saber que `-k` não leva valor, `sudo -k make` teria de desistir em "sudo".
 *
 * > [!warning] Uma flag ausente das duas listas é DESCONHECIDA, e desconhecida
 * > faz a leitura parar no wrapper (ver `programName`). É por isso que
 * > `-S`/`--split-string` do `env` fica fora de propósito: o valor dele é a
 * > linha de comando inteira em um token só, que este parser não separa.
 */
interface WrapperFlags {
  /** Consomem o próximo token como valor. */
  value: Set<string>;
  /** Não consomem nada. */
  bare: Set<string>;
}

const flags = (value: string[], bare: string[]): WrapperFlags => ({
  value: new Set(value),
  bare: new Set(bare),
});

/**
 * Prefixos que não são o programa, e sim como ele foi chamado.
 *
 * `sudo vim` é o `vim` na tela; dizer "sudo está no controle" seria trocar o
 * nome do programa pelo nome da permissão.
 */
const WRAPPERS = new Map<string, WrapperFlags>([
  [
    "sudo",
    flags(
      [
        "-u", "--user",
        "-g", "--group",
        "-p", "--prompt",
        "-C", "--close-from",
        "-h", "--host",
        "-R", "--chroot",
        "-D", "--chdir",
        "-T", "--command-timeout",
        "-U", "--other-user",
        "-r", "--role",
        "-t", "--type",
      ],
      [
        "-A", "--askpass",
        "-b", "--background",
        "-B", "--bell",
        "-E", "--preserve-env",
        "-H", "--set-home",
        "-i", "--login",
        "-K", "--remove-timestamp",
        "-k", "--reset-timestamp",
        "-n", "--non-interactive",
        "-N", "--no-update",
        "-P", "--preserve-groups",
        "-S", "--stdin",
        "-s", "--shell",
      ],
    ),
  ],
  ["doas", flags(["-u", "-a", "-C"], ["-L", "-n", "-s"])],
  [
    "env",
    flags(
      ["-u", "--unset", "-C", "--chdir"],
      ["-i", "--ignore-environment", "-0", "--null", "-v", "--debug"],
    ),
  ],
  ["command", flags([], ["-p", "-v", "-V"])],
  ["nohup", flags([], [])],
  [
    "time",
    flags(
      ["-f", "--format", "-o", "--output"],
      ["-p", "--portability", "-a", "--append", "-v", "--verbose"],
    ),
  ],
]);

type FlagStep = "bare" | "value" | "unknown";

/** Quanto um token de flag consome: só a si mesmo, o próximo também, ou sabe-se lá. */
function flagStep(known: WrapperFlags, token: string): FlagStep {
  if (token.startsWith("--")) {
    // `--user=app` traz o valor colado: seja a flag conhecida ou não, ela
    // nunca come o próximo token — e o próximo token é o programa.
    if (token.includes("=")) return "bare";
    if (known.value.has(token)) return "value";
    return known.bare.has(token) ? "bare" : "unknown";
  }
  const letters = token.slice(1);
  if (!letters) return "unknown";
  for (let i = 0; i < letters.length; i++) {
    const flag = `-${letters[i]}`;
    if (known.value.has(flag)) {
      // Num bundle, só a última letra pode levar valor: em `-uH app` o valor do
      // `-u` seria o "H", e em `-uapp` seria "app" colado. Ambíguo demais.
      return i === letters.length - 1 ? "value" : "unknown";
    }
    if (!known.bare.has(flag)) return "unknown";
  }
  return "bare";
}

/** `FOO=bar BAZ=qux nvim` — atribuição de ambiente vem antes do programa. */
function dropAssignments(words: string[]): string[] {
  let rest = words;
  while (rest.length > 0 && /^[A-Za-z_][A-Za-z0-9_]*=/.test(rest[0])) {
    rest = rest.slice(1);
  }
  return rest;
}

/**
 * O que sobra depois das flags do wrapper, ou `null` quando alguma delas é
 * ilegível — e aí quem responde é o wrapper, não um chute.
 */
function afterWrapperFlags(
  words: string[],
  known: WrapperFlags,
): string[] | null {
  let rest = dropAssignments(words);
  while (rest.length > 0) {
    const token = rest[0];
    if (token === "--") return dropAssignments(rest.slice(1));
    if (!token.startsWith("-")) break;
    const step = flagStep(known, token);
    if (step === "unknown") return null;
    rest = dropAssignments(rest.slice(step === "value" ? 2 : 1));
  }
  return rest;
}

/**
 * O nome do programa que está com a tela, a partir da linha de comando.
 *
 * Serve ao rótulo da linha colapsada: com um app de tela cheia rodando, dizer
 * "nvim está no controle" é infinitamente mais útil que "um app está usando a
 * tela" — o usuário reconhece o que ele mesmo abriu.
 *
 * > [!warning] É o comando que o usuário DIGITOU, não o processo em primeiro
 * > plano do tty. `git log` abre o `less` e esta função devolve "git".
 * >
 * > É impreciso e é honesto: foi `git log` que o usuário pediu, e é `git log`
 * > que ele vai reconhecer. A fidelidade real custaria um `tcgetpgrp` no fd do
 * > master no core — a infraestrutura existe (`PtyPool::line_echo` já faz esse
 * > acesso), mas é outra fatia, e esta não depende dela.
 *
 * O critério não é precisão absoluta, é nunca afirmar bobagem: diante de uma
 * flag de wrapper que não sabe ler, a leitura para e o nome do wrapper é a
 * resposta. "sudo" é impreciso e verdadeiro; devolver o operando de uma flag
 * seria errado com confiança, que num rótulo é a pior forma de errar — não se
 * parece com defeito, então ninguém desconfia.
 */
export function programName(command: string | null | undefined): string | null {
  if (!command) return null;
  let words = dropAssignments(command.trim().split(/\s+/).filter(Boolean));
  while (words.length > 1) {
    const known = WRAPPERS.get(basename(words[0]));
    if (!known) break;
    const rest = afterWrapperFlags(words.slice(1), known);
    // Sem nada legível depois do wrapper (flag desconhecida, ou flags que
    // consumiram a linha toda), o wrapper É o que o usuário digitou.
    if (!rest || rest.length === 0) break;
    words = rest;
  }
  const first = words[0];
  if (!first) return null;
  const name = basename(first);
  return name || null;
}

/** O último segmento de um caminho: `/usr/local/bin/nvim` vira `nvim`. */
function basename(word: string): string {
  const trimmed = word.replace(/\/+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("/") + 1);
}

/**
 * O prefixo do PRIMEIRO token, ou `null` se o caret não estiver nele.
 *
 * Só o front sabe onde está o caret, então é aqui que se decide se a lista de
 * comandos deve ser consultada — o core recebe `null` e nem olha o registro.
 * Passado o primeiro token quem responde é caminho ou argumento: oferecer
 * `pnpm` como valor de uma flag seria pior que não oferecer nada.
 */
export function commandPrefix(text: string, caret: number): string | null {
  const before = text.slice(0, caret);
  // O espaço à ESQUERDA é sinal, não separador: ` comando` é a convenção
  // `ignorespace` ("não guarde isto no histórico"), e quem a usa continua no
  // primeiro token.
  const typed = before.trimStart();
  if (typed.length === 0) return null;
  // Qualquer espaço DEPOIS do começo já encerrou o primeiro token.
  if (/\s/.test(typed)) return null;
  return typed;
}
