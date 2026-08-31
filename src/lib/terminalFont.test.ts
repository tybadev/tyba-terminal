import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "bun:test";

/**
 * A mesma saída não pode trocar de fonte ao virar cartão.
 *
 * O bug que originou este teste: `lsd --tree` desenhava os ícones certos
 * enquanto o comando rodava e virava tofu no instante em que terminava. Não era
 * a fonte faltando — era que a saída troca de renderizador no meio do caminho.
 * Enquanto executa, quem desenha é o xterm, com o stack declarado em
 * `TerminalView.tsx`; quando o comando fecha, o `BlockList` redesenha as MESMAS
 * linhas em React, com o `font-mono` do Tailwind. Dois stacks, e só um deles
 * conhecia a Nerd Font.
 *
 * Por isso o teste lê os arquivos de verdade em vez de um fixture: `styles.css`
 * é GERADO (`scripts/port_styles.py`, a partir do `tyba-design-system`) e um
 * fixture seria uma quarta cópia do stack — passaria verde enquanto os arquivos
 * reais divergissem, que é exatamente a falha que se quer impedir.
 *
 * O que ele NÃO garante é que o glifo existe: isso depende do `.woff2`
 * embarcado, e nenhuma leitura de CSS enxerga a tabela `cmap`. Ele garante a
 * única parte que já regrediu — que os três stacks continuam concordando.
 */

const HERE = import.meta.dir;
const CSS = readFileSync(join(HERE, "..", "styles.css"), "utf8");
const TERMINAL_VIEW = readFileSync(
  join(HERE, "..", "components", "TerminalView.tsx"),
  "utf8",
);

const NERD = "Symbols Nerd Font Mono";
const PRIMARY = "JetBrains Mono";

/** O valor de uma custom property, sem aspas e sem espaço supérfluo. */
function token(css: string, name: string): string {
  const found = css.match(new RegExp(`^\\s*${name}:\\s*([^;]+);`, "m"));
  if (!found) throw new Error(`token ausente: ${name}`);
  return found[1].replace(/['"]/g, "").replace(/\s+/g, " ").trim();
}

/** O stack que o xterm recebe na construção do `Terminal`. */
function xtermStack(source: string): string {
  const found = source.match(/fontFamily:\s*\n?\s*'([^']+)'/);
  if (!found) throw new Error("fontFamily do xterm não encontrado");
  return found[1].replace(/['"]/g, "").replace(/\s+/g, " ").trim();
}

/** Os dois stacks mono do CSS mais o do xterm, pelo nome que aparece no erro. */
const STACKS: Array<[string, () => string]> = [
  // Usado por tudo que é "de máquina" e escreve `font-family` direto.
  ["--tyba-font-mono", () => token(CSS, "--tyba-font-mono")],
  // A ponte do Tailwind: é ele que a classe `font-mono` do BlockList resolve.
  ["--font-mono", () => token(CSS, "--font-mono")],
  ["xterm fontFamily", () => xtermStack(TERMINAL_VIEW)],
];

describe("stack mono do terminal", () => {
  for (const [name, read] of STACKS) {
    it(`${name} carrega a Nerd Font`, () => {
      expect(read()).toContain(NERD);
    });

    // Ordem importa e não é detalhe: a Nerd Font Mono tem métrica própria e
    // cobre latim. Colocada ANTES da JetBrains, ela sequestraria o texto comum
    // e a linha inteira mudaria de desenho — o oposto do que se quer.
    it(`${name} pede a Nerd Font depois da JetBrains, não antes`, () => {
      const stack = read();
      expect(stack.indexOf(PRIMARY)).toBeGreaterThanOrEqual(0);
      expect(stack.indexOf(PRIMARY)).toBeLessThan(stack.indexOf(NERD));
    });

    // O genérico existe para a máquina sem nenhuma das duas instaladas. Sem ele
    // o navegador cai na fonte proporcional do sistema e a coluna quebra.
    it(`${name} termina em monospace`, () => {
      expect(read().endsWith("monospace")).toBe(true);
    });
  }

  // A face precisa estar declarada em algum lugar: nomear a família num stack
  // sem `@font-face` correspondente é pedir uma fonte que só existe na máquina
  // de quem já a instalou por fora.
  it("a família está declarada com um @font-face próprio", () => {
    const face = readFileSync(join(HERE, "..", "fonts.css"), "utf8");
    expect(face).toContain(NERD);
    expect(face).toMatch(/url\([^)]*SymbolsNerdFontMono[^)]*\)/);
  });

  // A fonte PADRÃO precisa vir embarcada — numa máquina sem ela instalada, o
  // app media com uma métrica (a fonte declarada) e desenhava com outra (o
  // fallback que o navegador escolhe). Os quatro cortes cobrem o que o
  // terminal realmente desenha: texto normal, negrito, itálico do prompt e
  // negrito+itálico.
  it("a JetBrains Mono vem embarcada nos quatro cortes — regular, bold, italic, bold italic", () => {
    const face = readFileSync(join(HERE, "..", "fonts.css"), "utf8");
    expect(face).toContain(PRIMARY);
    expect(face).toMatch(/url\([^)]*JetBrainsMono-Regular\.woff2[^)]*\)/);
    expect(face).toMatch(/url\([^)]*JetBrainsMono-Bold\.woff2[^)]*\)/);
    expect(face).toMatch(/url\([^)]*JetBrainsMono-Italic\.woff2[^)]*\)/);
    expect(face).toMatch(/url\([^)]*JetBrainsMono-BoldItalic\.woff2[^)]*\)/);
  });

  it("cada corte da JetBrains Mono declara o peso e o estilo certos — senão o browser sintetiza o negrito/itálico", () => {
    const face = readFileSync(join(HERE, "..", "fonts.css"), "utf8");
    const blockFor = (file: string) => {
      const found = face.match(
        new RegExp(`@font-face\\s*{[^}]*${file}[^}]*}`, "s"),
      );
      if (!found) throw new Error(`bloco @font-face ausente para ${file}`);
      return found[0];
    };
    expect(blockFor("JetBrainsMono-Regular")).toMatch(/font-weight:\s*400/);
    expect(blockFor("JetBrainsMono-Regular")).toMatch(/font-style:\s*normal/);
    expect(blockFor("JetBrainsMono-Bold")).toMatch(/font-weight:\s*700/);
    expect(blockFor("JetBrainsMono-Bold")).toMatch(/font-style:\s*normal/);
    expect(blockFor("JetBrainsMono-Italic")).toMatch(/font-weight:\s*400/);
    expect(blockFor("JetBrainsMono-Italic")).toMatch(/font-style:\s*italic/);
    expect(blockFor("JetBrainsMono-BoldItalic")).toMatch(/font-weight:\s*700/);
    expect(blockFor("JetBrainsMono-BoldItalic")).toMatch(/font-style:\s*italic/);
  });

  // OFL exige que a licença acompanhe a fonte redistribuída.
  it("a licença OFL acompanha os woff2 embarcados", () => {
    const licensePath = join(HERE, "..", "assets", "fonts", "OFL.txt");
    const license = readFileSync(licensePath, "utf8");
    expect(license).toContain("SIL OPEN FONT LICENSE");
  });
});
