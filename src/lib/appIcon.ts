/**
 * Ícone do app (Configurações → Aparência). O estado mora no core: aqui só a
 * lista que a grade desenha e a navegação por teclado do grupo de rádios.
 *
 * `APP_ICONS` é espelho de `AppIcon::ALL` em `src-tauri/src/app_icon.rs` — o
 * teste `ids_match_the_front` de lá falha se os dois divergirem.
 */
export const APP_ICONS = [
  "blackout",
  "aurora",
  "filamento",
  "assinatura",
  "fosforo",
  "violeta",
  "grafite",
  "papel",
] as const;

export type AppIconId = (typeof APP_ICONS)[number];

/** Nomes próprios das variantes: iguais em todo idioma. */
export const APP_ICON_NAMES: Record<AppIconId, string> = {
  blackout: "BLACKOUT",
  aurora: "Aurora",
  filamento: "Filamento",
  assinatura: "Assinatura",
  fosforo: "Fósforo",
  violeta: "Violeta",
  grafite: "Grafite",
  papel: "Papel",
};

export function isAppIconId(v: unknown): v is AppIconId {
  return typeof v === "string" && (APP_ICONS as readonly string[]).includes(v);
}

/**
 * Próximo índice num grupo de rádios (WAI-ARIA): setas para trás/frente com
 * volta, Home/End nas pontas. `null` para tecla que o grupo não trata.
 */
export function radioStep(index: number, key: string, count: number): number | null {
  switch (key) {
    case "ArrowRight":
    case "ArrowDown":
      return (index + 1) % count;
    case "ArrowLeft":
    case "ArrowUp":
      return (index - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}
