// Placa da opção G (quadro "TYBA Vidro Revisão", 2026-11-02): o que flutua
// por cima das peças — diálogo, toast, cartão sobre o terminal. Raio 10, fio
// inteiro e sombra longa com o brilho na aresta de cima. A sombra lê
// `--tyba-plate-shadow` quando o tema define; sem ele, cai na aresta e na
// sombra grande de sempre. O `!` vence o `shadow-*` que o componente base
// (Dialog, Toast) já traz.
export const PLATE =
  "rounded-[10px] border [box-shadow:var(--tyba-plate-shadow,var(--tyba-edge),var(--tyba-shadow-lg))]!";

/** O fio da placa quando ela não tem cor de risco própria. No claro do
 *  TYBA é a aresta branca do vidro do Muriki (`--tyba-mk-glass-edge`). */
export const PLATE_LINE = "border-[var(--tyba-mk-glass-edge,var(--tyba-line))]";

/** Painel (modal): o acrílico do cromo. No claro do TYBA, o vidro
 *  `is-float` do Muriki (`.tyba-mk-glass`). */
export const PLATE_ACRYLIC =
  "tyba-mk-glass is-float bg-[var(--tyba-glass-section)] backdrop-blur-[28px] backdrop-saturate-150";

/** Flutuante sobre texto (toast, cartão no terminal): quase opaco (97%). No
 *  claro do TYBA, o vidro `is-float` do Muriki. */
export const PLATE_FLOAT = "tyba-mk-glass is-float bg-[var(--tyba-glass-float)] backdrop-blur-xl";

/** Fio de baixo entre faixas, na cor do fio da G. */
export const LINE_B = "shadow-[inset_0_-1px_0_var(--tyba-line)]";
