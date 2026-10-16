/**
 * A linha que fica no lugar da barra de configuração enquanto o nó roda
 * (quadro: "tarefa: subir ambiente · ~/pasta"): a primeira linha com texto
 * da tarefa ou do comando. Vazia quando não há nada escrito.
 */
export function firstLine(text: string): string {
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (trimmed) return trimmed;
  }
  return "";
}
