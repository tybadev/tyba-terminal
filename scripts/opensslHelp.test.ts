import { describe, expect, it } from "bun:test";

import { parseHelp, uniao } from "./opensslHelp";

describe("leitura do `openssl help`", () => {
  it("traz os subcomandos padrão da seção certa", () => {
    const saida = ["Standard commands", "asn1parse  ca  ciphers", "dgst  enc"].join("\n");
    expect(parseHelp(saida)).toEqual([
      { nome: "asn1parse", padrao: true },
      { nome: "ca", padrao: true },
      { nome: "ciphers", padrao: true },
      { nome: "dgst", padrao: true },
      { nome: "enc", padrao: true },
    ]);
  });

  it("nome de digest e de cifra não vira subcomando padrão", () => {
    // As duas listas são de ALGORITMO, não de comando: `openssl sha256` existe,
    // mas quem quer entender vai no `dgst` — e é isso que decide se a entrada
    // ganha descrição escrita à mão ou não.
    const saida = [
      "Standard commands",
      "dgst",
      "",
      "Message Digest commands (see the `dgst' command for more details)",
      "sha256  sha512",
      "",
      "Cipher commands (see the `enc' command for more details)",
      "aes-256-cbc",
    ].join("\n");
    expect(parseHelp(saida)).toEqual([
      { nome: "dgst", padrao: true },
      { nome: "sha256", padrao: false },
      { nome: "sha512", padrao: false },
      { nome: "aes-256-cbc", padrao: false },
    ]);
  });

  it("o preâmbulo antes da primeira seção não vira subcomando", () => {
    // Os dois imprimem algo antes da lista, e o do LibreSSL é uma mensagem de
    // ERRO: ele não reconhece `help`, avisa, e mesmo assim publica os comandos
    // (e sai com código diferente de zero). Sem esta guarda, `invalid` e
    // `command.` entrariam na base como subcomandos de `openssl`.
    const saida = [
      "openssl:Error: 'help' is an invalid command.",
      "",
      "Standard commands",
      "asn1parse  ca",
    ].join("\n");
    expect(parseHelp(saida)).toEqual([
      { nome: "asn1parse", padrao: true },
      { nome: "ca", padrao: true },
    ]);
  });
});

describe("união das implementações", () => {
  const libre = { rotulo: "LibreSSL", achados: parseHelp("Standard commands\ndgst  certhash") };
  const ossl = { rotulo: "OpenSSL", achados: parseHelp("Standard commands\ndgst  list") };

  it("o que existe nos dois não é marcado", () => {
    expect(uniao([libre, ossl])).toContainEqual({ nome: "dgst", padrao: true });
  });

  it("o que existe em um só carrega o rótulo dele", () => {
    // `list` é O comando de descoberta do OpenSSL 3 e não existe no LibreSSL;
    // `certhash` é o inverso. Oferecer sem dizer de onde é faz alguém rodar o
    // que a instalação dele não tem.
    const r = uniao([libre, ossl]);
    expect(r).toContainEqual({ nome: "list", padrao: true, somenteEm: "OpenSSL" });
    expect(r).toContainEqual({ nome: "certhash", padrao: true, somenteEm: "LibreSSL" });
  });

  it("recusa gerar a base a partir de uma implementação só", () => {
    // O modo de falhar que isto impede é SILENCIOSO: com uma fonte só a união
    // ainda devolve uma lista boa, e ninguém percebe que faltam 16 comandos até
    // um usuário reclamar que `openssl list` não aparece. Regenerar numa
    // máquina que só tem um dos dois tem que parar aqui.
    expect(() => uniao([libre])).toThrow(/duas implementações/);
  });

  it("padrão em qualquer uma das duas vale como padrão", () => {
    // Hoje nenhum nome diverge — conferido nas duas instalações. O contrato
    // fica preso mesmo assim: se um dia um comando passar a ser listado como
    // algoritmo numa das implementações, ele NÃO pode perder a descrição
    // escrita à mão por causa disso.
    const comoAlgoritmo = {
      rotulo: "A",
      achados: parseHelp("Cipher commands\nrand"),
    };
    const comoComando = { rotulo: "B", achados: parseHelp("Standard commands\nrand") };
    expect(uniao([comoAlgoritmo, comoComando])[0].padrao).toBe(true);
    expect(uniao([comoComando, comoAlgoritmo])[0].padrao).toBe(true);
  });
});
