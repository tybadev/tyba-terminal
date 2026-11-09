/**
 * Contraste mínimo da saída do terminal.
 *
 * Programa que força cor RGB ou 256 (`bat` com Monokai, `delta`, `glow`,
 * `lazygit`) não sabe o fundo do TYBA: o `#F8F8F2` do Monokai some sobre o
 * papel branco do tema claro. O xterm resolve com `minimumContrastRatio`; os
 * blocos pintam a saída em HTML por conta própria e usam esta mesma conta.
 *
 * É a conta do xterm (`rgba.ensureContrastRatio` em `common/Color.ts`), portada
 * para os dois lados darem a mesma cor: luminância relativa da WCAG, e a cor que
 * não passa anda 10% por passo para o preto (fundo claro) ou para o branco
 * (fundo escuro), escalando os três canais juntos — o matiz fica.
 */

export type Rgb = [number, number, number];

/** O padrão do VS Code: WCAG AA para texto normal. */
export const MIN_CONTRAST = 4.5;

/** `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()` e `rgba()`; o resto é `null`. */
export function parseRgb(css: string): Rgb | null {
  const value = css.trim();
  const hex = /^#([0-9a-f]{3,8})$/i.exec(value)?.[1];
  if (hex) {
    if (hex.length === 3 || hex.length === 4) {
      return [0, 1, 2].map((i) => parseInt(hex[i] + hex[i], 16)) as Rgb;
    }
    if (hex.length === 6 || hex.length === 8) {
      return [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb;
    }
    return null;
  }
  const fn = /^rgba?\(\s*(\d{1,3})[\s,]+(\d{1,3})[\s,]+(\d{1,3})\b/i.exec(value);
  if (fn) return [Number(fn[1]), Number(fn[2]), Number(fn[3])];
  return null;
}

function luminance([r, g, b]: Rgb): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return lin(r) * 0.2126 + lin(g) * 0.7152 + lin(b) * 0.0722;
}

export function contrastRatio(a: Rgb, b: Rgb): number {
  const la = luminance(a);
  const lb = luminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

function darken(fg: Rgb, bg: Rgb, ratio: number): Rgb {
  let [r, g, b] = fg;
  while (contrastRatio([r, g, b], bg) < ratio && (r > 0 || g > 0 || b > 0)) {
    r -= Math.ceil(r * 0.1);
    g -= Math.ceil(g * 0.1);
    b -= Math.ceil(b * 0.1);
  }
  return [r, g, b];
}

function lighten(fg: Rgb, bg: Rgb, ratio: number): Rgb {
  let [r, g, b] = fg;
  while (contrastRatio([r, g, b], bg) < ratio && (r < 255 || g < 255 || b < 255)) {
    r = Math.min(255, r + Math.ceil((255 - r) * 0.1));
    g = Math.min(255, g + Math.ceil((255 - g) * 0.1));
    b = Math.min(255, b + Math.ceil((255 - b) * 0.1));
  }
  return [r, g, b];
}

/**
 * A cor de frente ajustada até `ratio` contra `bg`, ou `null` se já passa.
 *
 * Vai primeiro para o lado oposto ao fundo; se nem o extremo alcança, tenta o
 * outro lado e fica com o que contrastar mais — igual ao xterm.
 */
export function ensureContrast(fg: Rgb, bg: Rgb, ratio = MIN_CONTRAST): Rgb | null {
  if (contrastRatio(fg, bg) >= ratio) return null;
  const darker = luminance(fg) < luminance(bg);
  const first = darker ? darken(fg, bg, ratio) : lighten(fg, bg, ratio);
  if (contrastRatio(first, bg) >= ratio) return first;
  const second = darker ? lighten(fg, bg, ratio) : darken(fg, bg, ratio);
  return contrastRatio(first, bg) > contrastRatio(second, bg) ? first : second;
}

/**
 * O fundo do tema com alfa zero, para o xterm.
 *
 * A folha do terminal é transparente (quem pinta é a peça em volta), mas o
 * xterm mede o contraste pelo RGB do `background` e ignora o alfa. Com
 * `rgba(0, 0, 0, 0)` ele mediria contra PRETO e, no tema claro, clarearia o
 * texto escuro sobre o papel branco. Carregar o RGB real com alfa zero mantém a
 * folha transparente e a conta certa.
 */
export function transparentOf(css: string | undefined): string {
  const rgb = css ? parseRgb(css) : null;
  if (!rgb) return "rgba(0, 0, 0, 0)";
  return `rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, 0)`;
}
