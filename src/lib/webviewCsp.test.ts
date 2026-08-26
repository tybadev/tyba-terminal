import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "bun:test";

/**
 * `'unsafe-inline'` sozinho NÃO libera estilo em runtime neste app.
 *
 * O Tauri reescreve o CSP e injeta **nonce** nos estilos dele. Pela
 * especificação do CSP, a presença de um nonce numa diretiva **anula** o
 * `'unsafe-inline'` dela — a fonte permitida passa a ser só quem carrega o
 * nonce. Os `<style>` do próprio Tauri carregam; os que uma biblioteca cria em
 * tempo de execução, não.
 *
 * Quem cria `<style>` em runtime aqui é o CodeMirror: é assim que ele instala o
 * CSS BASE dele — `white-space: pre` no conteúdo, `display: flex` no scroller,
 * o posicionamento das linhas. Bloqueado esse `<style>`, o editor abre com o
 * conteúdo deslocado do gutter e o texto some da área visível. Medido em
 * 26/08/2026 numa sonda de boot dentro do app: sem
 * `dangerousDisableAssetCspModification`, um `<style>` criado por JavaScript
 * volta com `sheet` nulo — o WebKit recusa parsear.
 *
 * Este teste existe porque o nome da opção convida a apagá-la. Quem ler
 * `dangerous…` num arquivo de configuração e quiser limpar não tem como
 * adivinhar que ela é o que faz o editor funcionar — e a falha que ela causa
 * não parece de CSP, parece bug de layout.
 *
 * O que ele NÃO faz é afrouxar segurança: `default-src` e `script-src`
 * continuam intactos, e a diretiva liberada é exatamente a que o próprio
 * arquivo já declarava.
 */
describe("CSP do webview", () => {
  const conf = JSON.parse(
    readFileSync(
      join(import.meta.dir, "..", "..", "src-tauri", "tauri.conf.json"),
      "utf8",
    ),
  ) as {
    app: {
      security: { csp?: string; dangerousDisableAssetCspModification?: string[] };
    };
  };
  const seguranca = conf.app.security;

  it("declara `unsafe-inline` para estilo", () => {
    expect(seguranca.csp).toContain("style-src");
    expect(seguranca.csp).toContain("'unsafe-inline'");
  });

  it("impede o Tauri de pôr nonce em style-src, senão o unsafe-inline não vale", () => {
    expect(seguranca.dangerousDisableAssetCspModification).toContain("style-src");
  });

  it("não desliga a modificação inteira — só a diretiva de estilo", () => {
    // `true` desligaria para TODAS as diretivas, inclusive `script-src`. A
    // diferença entre uma lista com um item e um booleano é a diferença entre
    // liberar CSS e liberar execução de código.
    const alvo = seguranca.dangerousDisableAssetCspModification;
    expect(Array.isArray(alvo)).toBe(true);
    expect(alvo).toEqual(["style-src"]);
  });

  it("o script continua trancado", () => {
    // A regressão que importaria de verdade: alguém "resolver" um problema de
    // script relaxando o CSP. `default-src 'self'` cobre `script-src` por
    // herança, e nada aqui pode abrir isso.
    expect(seguranca.csp).toContain("default-src 'self'");
    expect(seguranca.csp).not.toContain("script-src");
    expect(seguranca.csp).not.toContain("'unsafe-eval'");
  });
});
