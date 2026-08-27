/**
 * Extrai o ESTÁTICO de um spec do Fig e devolve linhas prontas para a tabela.
 *
 * O spec compilado é um módulo ES que exporta o objeto — subcomandos, flags,
 * descrições, e também `generators`, funções e templates. Só as três primeiras
 * coisas atravessam daqui.
 *
 * `generators` são descartados por decisão registrada em
 * `decisions/2026-08-26-a-base-de-specs-do-fig-e-mit-e-alcancavel`: eles rodam
 * comando no shell a cada tecla, e importar isso seria deixar uma base de
 * terceiro executar no terminal do dono. **Não são convertidos nem adiados —
 * são descartados**, e o teste afirma que a palavra nem aparece na saída. Dado
 * morto na tabela vira alguém tentando executá-lo depois, achando que estava
 * previsto.
 */

/** Uma linha da tabela `command_spec`. */
export interface Linha {
  command: string;
  /** Caminho depois do comando: `commit`, `container ls`, `--force`. */
  path: string;
  kind: "subcommand" | "option";
  description?: string;
}

/**
 * Teto da descrição, em caracteres.
 *
 * Ela é o que domina o peso da semente, e a lista tem uma linha por item —
 * texto além disso não cabe na tela e só pesa no banco do usuário.
 */
const MAX_DESCRICAO = 120;

/**
 * O nome, quando o Fig declara alias.
 *
 * `["-f", "--force"]` é como ele escreve flag com apelido. Juntar viraria
 * `-f,--force`, que não é um nome e nunca casaria com o que se digita.
 */
function nome(bruto: unknown): string {
  if (Array.isArray(bruto)) return typeof bruto[0] === "string" ? bruto[0] : "";
  return typeof bruto === "string" ? bruto : "";
}

/**
 * A descrição, cortada por CARACTERE.
 *
 * `slice` em JavaScript opera em unidades UTF-16, então cortar aqui não parte
 * um acento ao meio como um corte por byte partiria — e é isso que impede lixo
 * no banco.
 */
/// Quem decide "tem descrição?" é ESTA função, e só ela. O chamador compara
/// com `undefined` em vez de repetir a regra por truthiness — duas guardas
/// para o mesmo fato protegem uma à outra e viram código morto que nenhum
/// teste consegue derrubar.
function descricao(bruto: unknown): string | undefined {
  if (typeof bruto !== "string" || bruto.length === 0) return undefined;
  return bruto.length > MAX_DESCRICAO ? bruto.slice(0, MAX_DESCRICAO) : bruto;
}

interface Cru {
  name?: unknown;
  description?: unknown;
  subcommands?: Cru[];
  options?: { name?: unknown; description?: unknown }[];
}

/**
 * As linhas de um spec, já sem nada que execute.
 *
 * O comando raiz **não** vira linha: a tabela responde "o que vem depois de
 * `git`", e o `git` em si é a coluna `command`.
 */
export function extrai(spec: Cru): Linha[] {
  const comando = nome(spec?.name);
  if (!comando) return [];
  const linhas: Linha[] = [];

  const desce = (item: Cru, prefixo: string[]) => {
    for (const sub of item.subcommands ?? []) {
      const n = nome(sub?.name);
      if (!n) continue;
      const caminho = [...prefixo, n];
      const d = descricao(sub?.description);
      linhas.push({
        command: comando,
        path: caminho.join(" "),
        kind: "subcommand",
        ...(d !== undefined ? { description: d } : {}),
      });
      desce(sub, caminho);
    }
    for (const op of item.options ?? []) {
      const n = nome(op?.name);
      if (!n) continue;
      const d = descricao(op?.description);
      linhas.push({
        command: comando,
        // Flag de subcomando carrega o caminho dele: `container ls --all` é
        // outra coisa que `--all` solto na raiz.
        path: [...prefixo, n].join(" "),
        kind: "option",
        ...(d !== undefined ? { description: d } : {}),
      });
    }
  };

  desce(spec, []);
  return linhas;
}
