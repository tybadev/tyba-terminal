/**
 * Estado de carregamento: a regra pura por trás do `LoadingState`.
 *
 * O padrão (feature `carregamento`): esqueleto no formato final, filamento só
 * enquanto há trabalho real, a ETAPA real em vez de "Carregando…", e — passado
 * o tempo normal da operação — o motivo provável e uma ação. A etapa é feita
 * de fatos que o core entregou (caminho, host, sha, total); a UI só os
 * escreve na língua do produto.
 */

/** As operações que carregam com o padrão. Cada uma tem o seu "normal". */
export type LoadOp =
  | "docker"
  | "files"
  | "filesSearch"
  | "editor"
  | "shell"
  | "forge"
  | "diff"
  | "merge"
  | "gitLog"
  | "subagent";

/**
 * A partir de quanto tempo a operação está "demorando mais que o normal".
 *
 * Medido no uso, não inventado: o `docker ps` local responde em <1 s com o
 * daemon de pé (5 s cobre o Docker Desktop ainda subindo); o `.zshrc` do dono
 * leva 1,4 s (ver `boxAcceptsTyping`), então 5 s já é anormal; `gh` é sempre
 * rede, e a API do GitHub passa de 3-4 s em dia ruim. Remoto ganha folga: é
 * SSH + o comando no servidor.
 */
export const SLOW_AFTER_MS: Record<LoadOp, { local: number; remote: number }> = {
  docker: { local: 5_000, remote: 10_000 },
  files: { local: 4_000, remote: 10_000 },
  filesSearch: { local: 3_000, remote: 10_000 },
  editor: { local: 3_000, remote: 10_000 },
  shell: { local: 5_000, remote: 15_000 },
  forge: { local: 8_000, remote: 8_000 },
  diff: { local: 4_000, remote: 4_000 },
  merge: { local: 6_000, remote: 6_000 },
  gitLog: { local: 4_000, remote: 4_000 },
  subagent: { local: 3_000, remote: 3_000 },
};

export function slowAfterMs(op: LoadOp, remote = false): number {
  const limits = SLOW_AFTER_MS[op];
  return remote ? limits.remote : limits.local;
}

/** Passou do normal? Sem início conhecido, nunca: não há o que medir. */
export function isSlow(
  startedAt: number | null | undefined,
  now: number,
  op: LoadOp,
  remote = false,
): boolean {
  if (startedAt == null || !Number.isFinite(startedAt)) return false;
  return now - startedAt >= slowAfterMs(op, remote);
}

/** Segundos inteiros desde o início, para "(6 s)". Relógio para trás vira 0. */
export function elapsedSeconds(startedAt: number, now: number): number {
  return Math.max(0, Math.floor((now - startedAt) / 1000));
}

/**
 * Duração curta e legível: "0,8 s" abaixo de 10 s (uma casa), "12 s" acima.
 * A vírgula vem do idioma.
 */
export function formatSeconds(ms: number, lang: string): string {
  const s = Math.max(0, ms) / 1000;
  const digits = s < 10 ? 1 : 0;
  const n = new Intl.NumberFormat(lang, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(digits === 0 ? Math.floor(s) : Math.floor(s * 10) / 10);
  return `${n} s`;
}

export interface LoadProgress {
  done: number;
  total: number;
  /** 0–100, inteiro. */
  percent: number;
}

/**
 * Progresso numérico SÓ quando o core sabe o total. Total ausente, zero ou
 * inválido devolve `null` — a UI não desenha barra nem "N de M" chutados.
 */
export function progressOf(
  done: number | null | undefined,
  total: number | null | undefined,
): LoadProgress | null {
  if (total == null || !Number.isFinite(total) || total <= 0) return null;
  if (done == null || !Number.isFinite(done)) return null;
  const d = Math.min(Math.max(0, Math.floor(done)), Math.floor(total));
  const t = Math.floor(total);
  return { done: d, total: t, percent: Math.round((d / t) * 100) };
}

/** Os 7 primeiros do sha, como o git mostra. */
export function shortSha(sha: string): string {
  return sha.trim().slice(0, 7);
}

/** Um pedaço do texto da etapa; `em` = o fato (caminho, host, sha), aceso. */
export interface StepPart {
  text: string;
  em: boolean;
}

/**
 * Parte o texto já traduzido nos fatos que ele cita, para acendê-los (mockup:
 * "lendo **~/poc-git** · 1.240 arquivos"). Cada valor é aceso na primeira vez
 * em que aparece; vazio é ignorado. Fica fora do i18n de propósito: a ordem
 * das palavras muda com o idioma, o fato não.
 */
export function splitEmphasis(text: string, values: readonly string[]): StepPart[] {
  const wanted = values.filter((v) => v.length > 0);
  const parts: StepPart[] = [];
  let rest = text;
  for (;;) {
    let best: { at: number; value: string } | null = null;
    for (const v of wanted) {
      const at = rest.indexOf(v);
      if (at >= 0 && (best === null || at < best.at || (at === best.at && v.length > best.value.length))) {
        best = { at, value: v };
      }
    }
    if (best === null) break;
    if (best.at > 0) parts.push({ text: rest.slice(0, best.at), em: false });
    parts.push({ text: best.value, em: true });
    rest = rest.slice(best.at + best.value.length);
    wanted.splice(wanted.indexOf(best.value), 1);
  }
  if (rest.length > 0) parts.push({ text: rest, em: false });
  return parts;
}

/** O que o core devolve sobre o shell que está subindo (`session_shell_boot`). */
export interface ShellBoot {
  /** `zsh`, `bash`… */
  shell: string;
  /** O rc do usuário que o shell está lendo, já com `~`. `null` = não se sabe. */
  rc: string | null;
  /** Epoch em ms do spawn. */
  started_at_ms: number;
}

/** A etapa do shell subindo, como chave de i18n + parâmetros. */
export interface BootStep {
  key: "shellBootRc" | "shellBootShell" | "shellBootRemote" | "shellBootOpening";
  params: Record<string, string>;
}

/**
 * Onde o shell está: o PTY já existe (terminal aberto) e falta o primeiro
 * prompt — ou seja, o rc. A integração do TYBA é o próprio primeiro prompt, e
 * quando ela chega a linha deixa de esperar: não há etapa para mostrar.
 *
 * Remoto: o core não sabe qual rc o servidor lê; diz só onde.
 */
export function shellBootStep(boot: ShellBoot | null, host: string | null): BootStep {
  if (host) return { key: "shellBootRemote", params: { host } };
  if (!boot) return { key: "shellBootOpening", params: {} };
  if (boot.rc) return { key: "shellBootRc", params: { rc: boot.rc } };
  return { key: "shellBootShell", params: { shell: boot.shell } };
}
