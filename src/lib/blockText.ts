import type { Block } from "./ipc";

/**
 * Ctrl+C (130) e SIGPIPE (141) não são falha: são o usuário interrompendo e um
 * pipe fechando. Pintar isso de vermelho treinaria o olho a ignorar vermelho.
 */
export function failed(exitCode: number | null): boolean {
  return (
    exitCode !== null && exitCode !== 0 && exitCode !== 130 && exitCode !== 141
  );
}

/** Duração só a partir de 1s: abaixo disso o número não informa nada. */
export function duration(block: Block): string | null {
  const ms = block.finishedAtMs - block.startedAtMs;
  if (ms < 1000) return null;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.round(ms / 60_000)}min`;
}

/**
 * A saída do bloco como texto puro.
 *
 * Lê o MODELO, nunca o que está desenhado: o corpo do cartão corta em 200
 * linhas até o usuário expandir, e copiar do DOM devolveria menos do que o
 * bloco tem — silenciosamente, que é o pior jeito de errar numa cópia.
 *
 * Sem `\n` no fim de propósito: o destino comum é a linha de comando, e lá uma
 * quebra final é um Enter.
 */
export function blockOutput(block: Block): string {
  return block.lines.map((line) => line.text).join("\n");
}

/**
 * Cerca longa o bastante para a saída não escapar dela.
 *
 * CommonMark fecha o bloco na primeira linha com uma cerca de tamanho igual ou
 * maior; um `cat README.md` traz ``` na saída e partiria o bloco ao meio, com o
 * resto virando texto solto no meio da issue.
 */
function fenceFor(content: string): string {
  let longest = 0;
  for (const run of content.match(/`+/g) ?? []) {
    longest = Math.max(longest, run.length);
  }
  return "`".repeat(Math.max(3, longest + 1));
}

/**
 * O bloco como markdown, para colar em issue, PR ou chat.
 *
 * As notas (exit, truncagem, duração) são neutras de idioma: este texto sai do
 * app e vai ser lido por outra pessoa, num lugar que não tem a preferência de
 * idioma de quem copiou.
 */
export function blockMarkdown(block: Block): string {
  const output = blockOutput(block);
  const body = [block.command ? `$ ${block.command}` : "", output]
    .filter((part) => part.length > 0)
    .join("\n");
  const fence = fenceFor(body);

  const notes: string[] = [];
  if (failed(block.exitCode)) notes.push(`exit ${block.exitCode}`);
  if (block.truncated > 0) notes.push(`${block.truncated} lines omitted`);
  const took = duration(block);
  if (took) notes.push(took);

  const lines = [`${fence}console`, body, fence];
  if (notes.length > 0) lines.push(notes.join(" · "));
  return lines.join("\n");
}
