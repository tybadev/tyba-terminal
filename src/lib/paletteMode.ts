export type PaletteMode = "actions" | "files" | "sessions";

export const PALETTE_MODES: PaletteMode[] = ["actions", "files", "sessions"];

/// Tab avança (Ações → Arquivos → Sessões → Ações), Shift+Tab volta. Função
/// pura pra testar o ciclo sem montar a paleta.
export function nextPaletteMode(mode: PaletteMode, dir: 1 | -1 = 1): PaletteMode {
  const count = PALETTE_MODES.length;
  const index = PALETTE_MODES.indexOf(mode);
  const base = index < 0 ? 0 : index;
  return PALETTE_MODES[(((base + dir) % count) + count) % count];
}
