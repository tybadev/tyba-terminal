import { describe, expect, it } from "bun:test";

import { extrai, type Linha } from "./figExtract";

/** O formato que um spec do Fig tem depois de compilado. */
const spec = (extra: Record<string, unknown> = {}) => ({
  name: "demo",
  description: "Comando de demonstração",
  ...extra,
});

const caminhos = (linhas: Linha[]) => linhas.map((l) => `${l.kind}:${l.path}`);

describe("extração do spec do Fig", () => {
  it("traz subcomando e flag com a descrição", () => {
    const linhas = extrai(
      spec({
        subcommands: [{ name: "commit", description: "Grava as mudanças" }],
        options: [{ name: "--force", description: "Não pergunta" }],
      }),
    );
    expect(linhas).toEqual([
      { command: "demo", path: "commit", kind: "subcommand", description: "Grava as mudanças" },
      { command: "demo", path: "--force", kind: "option", description: "Não pergunta" },
    ]);
  });

  it("descarta `generators` sem deixar rastro", () => {
    // A regra do ADR: eles rodam comando no shell a cada tecla. Não são
    // convertidos nem adiados — são descartados. Um `generators` que
    // sobrevivesse como dado morto na tabela seria alguém tentando executá-lo
    // depois, achando que estava previsto.
    const linhas = extrai(
      spec({
        subcommands: [
          {
            name: "checkout",
            description: "Troca de branch",
            args: { name: "branch", generators: { script: ["git", "branch"] } },
          },
        ],
      }),
    );
    expect(linhas).toEqual([
      { command: "demo", path: "checkout", kind: "subcommand", description: "Troca de branch" },
    ]);
    expect(JSON.stringify(linhas)).not.toContain("generators");
    // Nenhuma chave além das quatro previstas: é isso que garante que nada
    // executável passou de carona. (Buscar a substring "script" não serve —
    // "description" contém "script".)
    for (const linha of linhas) {
      expect(Object.keys(linha).sort()).toEqual([
        "command",
        "description",
        "kind",
        "path",
      ]);
    }
  });

  it("nome em array vira o primeiro, não uma string colada", () => {
    // `["-f", "--force"]` é como o Fig declara alias de flag. Juntar viraria
    // `-f,--force`, que não é um nome e nunca casaria com o que se digita.
    const linhas = extrai(spec({ options: [{ name: ["-f", "--force"] }] }));
    expect(linhas[0].path).toBe("-f");
  });

  it("subcomando aninhado chega com o caminho inteiro", () => {
    // `docker container ls` — o caminho é o que a consulta usa como prefixo.
    const linhas = extrai(
      spec({
        subcommands: [
          { name: "container", subcommands: [{ name: "ls", description: "Lista" }] },
        ],
      }),
    );
    expect(caminhos(linhas)).toContain("subcommand:container ls");
  });

  it("flag de subcomando carrega o caminho dele", () => {
    // `docker container ls --all` é outra coisa que `--all` solto na raiz. Sem
    // o caminho, a flag de um subcomando apareceria ao completar qualquer
    // outro — e a lista passaria a oferecer o que não existe ali.
    const linhas = extrai(
      spec({
        subcommands: [
          {
            name: "container",
            subcommands: [{ name: "ls", options: [{ name: "--all" }] }],
          },
        ],
      }),
    );
    expect(caminhos(linhas)).toContain("option:container ls --all");
  });

  it("descrição longa é cortada sem partir caractere", () => {
    // Corte por byte partiria um acento ao meio e gravaria lixo no banco.
    const longa = "á".repeat(200);
    const linhas = extrai(spec({ subcommands: [{ name: "x", description: longa }] }));
    const d = linhas[0].description!;
    expect(d.length).toBeLessThanOrEqual(120);
    expect(d).toBe("á".repeat(120));
  });

  it("sem descrição não inventa string vazia", () => {
    // `""` no banco é diferente de "não tem": a lista mostraria uma coluna
    // vazia em vez de não mostrar coluna.
    const linhas = extrai(spec({ subcommands: [{ name: "x" }] }));
    expect(linhas[0].description).toBeUndefined();
  });

  it("descrição vazia é ausência, não string vazia", () => {
    // O Fig tem itens com `description: ""`. `""` no banco é diferente de "não
    // tem": a lista mostraria uma coluna vazia em vez de não mostrar coluna.
    const linhas = extrai(spec({ subcommands: [{ name: "x", description: "" }] }));
    expect(linhas[0].description).toBeUndefined();
    expect("description" in linhas[0]).toBe(false);
  });

  it("item sem nome é descartado em vez de virar linha vazia", () => {
    const linhas = extrai(spec({ subcommands: [{ description: "sem nome" }, { name: "ok" }] }));
    expect(caminhos(linhas)).toEqual(["subcommand:ok"]);
  });

  it("o próprio comando não vira linha", () => {
    // A tabela responde "o que vem DEPOIS de `demo`". O `demo` em si é a
    // coluna `command`, não uma entrada.
    const linhas = extrai(spec());
    expect(linhas).toEqual([]);
  });
});
