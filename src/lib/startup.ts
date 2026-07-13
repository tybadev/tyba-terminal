import type { StartupMode } from "../components/SettingsView";

/** Espelha `StartupMode::parse` no core: valor desconhecido ou ausente cai em
 * `resume`, que é o que um terminal faz — reabre onde você parou. */
export function parseStartupMode(raw: string | null | undefined): StartupMode {
  if (raw === "keep_layout") return "keep_layout";
  if (raw === "fresh") return "fresh";
  return "resume";
}
