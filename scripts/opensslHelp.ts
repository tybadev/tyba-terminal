/** Um subcomando anunciado pelo `openssl help`. */
export interface Achado {
  nome: string;
  /** `true` para "Standard commands"; `false` para nome de digest ou cifra. */
  padrao: boolean;
}

/**
 * Os cabeçalhos de seção, e o que cada seção contém.
 *
 * As duas últimas são listas de ALGORITMO, não de comando: `openssl sha256`
 * funciona, mas quem quer entender vai no `dgst` e no `enc` — que o próprio
 * cabeçalho já aponta. É por isso que elas não ganham descrição escrita à mão.
 */
const SECOES: { titulo: string; padrao: boolean }[] = [
  { titulo: "Standard commands", padrao: true },
  { titulo: "Message Digest commands", padrao: false },
  { titulo: "Cipher commands", padrao: false },
];

/** Os subcomandos que a saída do `openssl help` anuncia. */
export function parseHelp(saida: string): Achado[] {
  const achados: Achado[] = [];
  let secao: (typeof SECOES)[number] | undefined;
  for (const linha of saida.split("\n")) {
    const cabecalho = SECOES.find((s) => linha.startsWith(s.titulo));
    if (cabecalho) {
      secao = cabecalho;
      continue;
    }
    if (!secao || linha.trim() === "") continue;
    for (const nome of linha.trim().split(/\s+/)) {
      achados.push({ nome, padrao: secao.padrao });
    }
  }
  return achados;
}

/** O que uma implementação do `openssl` anuncia. */
export interface Fonte {
  rotulo: string;
  achados: Achado[];
}

/** Uma entrada da base, já reconciliada entre as implementações. */
export interface Entrada {
  nome: string;
  padrao: boolean;
  /** Preenchido quando o subcomando existe em uma implementação só. */
  somenteEm?: string;
}

/**
 * Reconcilia as implementações numa lista só.
 *
 * `openssl` é dois programas com o mesmo nome. A Apple embarca LibreSSL; Linux
 * e Homebrew embarcam OpenSSL. Eles divergem em 16 subcomandos — e entre os que
 * só o OpenSSL tem está o `list`, que é justamente o comando de DESCOBERTA
 * ("openssl list -commands"). Gerar a base a partir de um só faria a tabela
 * mentir para metade dos usuários, e o erro seria silencioso: a lista continua
 * respondendo, só que sem o que aquela máquina tem.
 */
export function uniao(fontes: Fonte[]): Entrada[] {
  if (fontes.length < 2) {
    throw new Error(
      "a base de `openssl` sai da união de duas implementações; " +
        `recebi ${fontes.length}. Instale o que falta antes de regenerar.`,
    );
  }
  const nomes = new Map<string, { padrao: boolean; onde: Set<string> }>();
  for (const fonte of fontes) {
    for (const achado of fonte.achados) {
      const atual = nomes.get(achado.nome);
      if (atual) {
        atual.padrao ||= achado.padrao;
        atual.onde.add(fonte.rotulo);
      } else {
        nomes.set(achado.nome, { padrao: achado.padrao, onde: new Set([fonte.rotulo]) });
      }
    }
  }
  return [...nomes.entries()]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([nome, { padrao, onde }]) => ({
      nome,
      padrao,
      ...(onde.size === 1 ? { somenteEm: [...onde][0] } : {}),
    }));
}
