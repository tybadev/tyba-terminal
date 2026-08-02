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

/**
 * O comando pede a tela limpa?
 *
 * Espelha `blocks::wipes_the_screen` no core, que é quem apaga a lista de
 * verdade. Aqui serve para a faixa ao vivo NÃO abrir: `clear` não tem saída
 * para mostrar, e abrir meio painel preto para depois esvaziar tudo é um
 * solavanco em cima de um comando cujo ponto é justamente sumir com as coisas.
 *
 * Só ele sozinho: `clear && ls` tem saída de verdade depois.
 */
export function wipesTheScreen(command: string | null): boolean {
  if (!command) return false;
  const trimmed = command.trim();
  return trimmed === "clear" || trimmed === "reset";
}

/** Duração só a partir de 1s: abaixo disso o número não informa nada. */
export function duration(block: Block): string | null {
  const ms = block.finishedAtMs - block.startedAtMs;
  if (ms < 1000) return null;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.round(ms / 60_000)}min`;
}

/**
 * O caminho encurtado para caber no header.
 *
 * Duas pastas finais em vez do caminho inteiro: o que responde "onde isto
 * rodou" é o fim, e o começo empurraria o comando para fora da linha. O
 * caminho completo fica no `title`.
 */
export function shortPath(path: string | null): string | null {
  if (!path) return null;
  const parts = path.split("/").filter(Boolean);
  if (parts.length === 0) return "/";
  const tail = parts.slice(-2).join("/");
  return parts.length > 2 ? `…/${tail}` : `/${tail}`;
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
  // Sem isto o bloco de um `vim` viraria um comando que não imprimiu nada.
  if (block.altScreen) notes.push("full-screen app, output not captured");
  if (block.truncated > 0) notes.push(`${block.truncated} lines omitted`);
  const took = duration(block);
  if (took) notes.push(took);

  const lines = [`${fence}console`, body, fence];
  if (notes.length > 0) lines.push(notes.join(" · "));
  return lines.join("\n");
}

/**
 * Vários blocos em markdown, na ordem da lista.
 *
 * Uma cerca por bloco, e não uma só com tudo dentro: cada comando tem o seu
 * recorte, que é a coisa inteira que os blocos existem para dar.
 */
export function blocksMarkdown(blocks: Block[]): string {
  return blocks.map(blockMarkdown).join("\n\n");
}
