export const SPINNER_FRAMES = [
  "⠋",
  "⠙",
  "⠹",
  "⠸",
  "⠼",
  "⠴",
  "⠦",
  "⠧",
  "⠇",
  "⠏",
] as const;

export const SPINNER_INTERVAL_MS = 120;

export interface WindowTitleOpts {
  base: string;
  running: boolean;
  attention: boolean;
  frame: number;
  reducedMotion: boolean;
}

export const windowTitle = ({
  base,
  running,
  attention,
  frame,
  reducedMotion,
}: WindowTitleOpts): string => {
  if (running) {
    const glyph = reducedMotion
      ? "✳"
      : SPINNER_FRAMES[frame % SPINNER_FRAMES.length];
    return `${glyph} ${base}`;
  }
  if (attention) return `● ${base}`;
  return base;
};
