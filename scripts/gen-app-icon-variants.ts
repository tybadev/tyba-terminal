/**
 * Gera as variantes do ícone do app que o usuário escolhe em Configurações →
 * Aparência → "Ícone do app" (feature `icone-app` no vault).
 *
 * Roda à mão, não no build — os arquivos gerados são commitados:
 *
 *   bun scripts/gen-app-icon-variants.ts
 *
 * Fonte: a marca (`src-tauri/icons/art/tyba-mark.svg`) e a tabela `VARIANTS`
 * abaixo, cópia da prancha aprovada pelo dono em 2026-10-14 (fundo, modo da
 * marca e brilho de cada variante). Não usa ferramenta nova: rasteriza com o
 * `tauri icon` (resvg) que o `gen-app-icons.ts` já usa.
 *
 * Saídas, para cada id:
 *
 * - `icons/app/src/<id>.svg` — o ladrilho 1024×1024 na grade de ícone do
 *   macOS: corpo de 824 centrado com 100 de margem transparente, cantos com
 *   raio de 22,37% do corpo. É a fonte versionada do PNG.
 * - `icons/app/<id>.png` — 1024×1024. O core embute (`include_bytes!`) e põe
 *   no Dock em tempo de execução.
 * - `icons/app/thumb/<id>.png` — 128×128 só do corpo (sem a margem), para a
 *   grade da tela de Configurações.
 */
import { mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const ROOT = resolve(import.meta.dir, "..");
const ICONS = join(ROOT, "src-tauri", "icons");
const OUT = join(ICONS, "app");

/** Grade do macOS em 1024. */
const CANVAS = 1024;
const BODY = 824;
const INSET = (CANVAS - BODY) / 2;
const RADIUS = Math.round(BODY * 0.2237);
/** Margem da marca dentro do corpo: 20% de cada lado, como na prancha. */
const PAD = Math.round(BODY * 0.2);
const MARK = BODY - 2 * PAD;
/** A borda interna da prancha é 1px num ladrilho de 168: mesma proporção. */
const EDGE = BODY / 168;
const THUMB = 128;

type Radial = { kind: "radial"; rx: number; ry: number; cx: number; cy: number; stops: Stop[] };
type Linear = { kind: "linear"; deg: number; stops: Stop[] };
type Stop = { at: number; color: string; opacity?: number };
/** Camadas de baixo para cima (o CSS lista de cima para baixo). */
type Layer = string | Radial | Linear;

type Variant = {
  id: string;
  layers: Layer[];
  /** `brand` = cores da marca; senão, uma cor sólida para todos os traços. */
  mark: "brand" | string;
  glow: boolean;
  edge: string;
  edgeOpacity: number;
};

const fade = (color: string, opacity: number, end: number): Stop[] => [
  { at: 0, color, opacity },
  { at: end, color, opacity: 0 },
];

const VARIANTS: Variant[] = [
  { id: "blackout", layers: ["#050505"], mark: "brand", glow: false, edge: "#ffffff", edgeOpacity: 0.1 },
  {
    id: "aurora",
    layers: [
      "#fafbff",
      { kind: "radial", rx: 0.9, ry: 0.7, cx: 0.15, cy: -0.05, stops: fade("#0f9d8c", 0.18, 0.6) },
      { kind: "radial", rx: 1.2, ry: 0.8, cx: 0.75, cy: -0.1, stops: fade("#7a4ce8", 0.22, 0.6) },
    ],
    mark: "brand",
    glow: false,
    edge: "#000000",
    edgeOpacity: 0.08,
  },
  {
    id: "filamento",
    layers: [
      {
        kind: "radial", rx: 0.7, ry: 0.7, cx: 0.5, cy: 0.45,
        stops: [{ at: 0, color: "#14141a" }, { at: 0.7, color: "#050505" }],
      },
    ],
    mark: "brand",
    glow: true,
    edge: "#ffffff",
    edgeOpacity: 0.1,
  },
  {
    id: "assinatura",
    layers: [
      {
        kind: "linear", deg: 135,
        stops: [
          { at: 0, color: "#7cc544" },
          { at: 0.22, color: "#f5c93b" },
          { at: 0.52, color: "#ec4899" },
          { at: 0.72, color: "#8b5cf6" },
          { at: 1, color: "#2dd4bf" },
        ],
      },
    ],
    mark: "#ffffff",
    glow: false,
    edge: "#000000",
    edgeOpacity: 0.06,
  },
  { id: "fosforo", layers: ["#050505"], mark: "#7cc544", glow: true, edge: "#7cc544", edgeOpacity: 0.25 },
  {
    id: "violeta",
    layers: [{ kind: "linear", deg: 160, stops: [{ at: 0, color: "#2a1659" }, { at: 1, color: "#12092b" }] }],
    mark: "#e9e2ff",
    glow: false,
    edge: "#ffffff",
    edgeOpacity: 0.1,
  },
  {
    id: "grafite",
    layers: [{ kind: "linear", deg: 160, stops: [{ at: 0, color: "#2a2a2e" }, { at: 1, color: "#141416" }] }],
    mark: "#f5f5f6",
    glow: false,
    edge: "#ffffff",
    edgeOpacity: 0.1,
  },
  {
    id: "papel",
    layers: [{ kind: "linear", deg: 160, stops: [{ at: 0, color: "#ffffff" }, { at: 1, color: "#ececf0" }] }],
    mark: "#121218",
    glow: false,
    edge: "#000000",
    edgeOpacity: 0.1,
  },
];

// O comentário do SVG da marca tem `--`, que é XML inválido.
const markSvg = readFileSync(join(ICONS, "art", "tyba-mark.svg"), "utf8").replace(/<!--[\s\S]*?-->\s*/g, "");
const markInner = markSvg.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "").trim();

function markBody(mode: string): string {
  if (mode === "brand") return markInner;
  // Cor sólida: some o `<defs>` dos gradientes e todo `url(#…)` vira a cor.
  return markInner.replace(/<defs>[\s\S]*?<\/defs>\s*/, "").replace(/url\(#[^)]+\)/g, mode);
}

const n = (v: number) => Number(v.toFixed(3));

function stops(list: Stop[]): string {
  return list
    .map((s) => `<stop offset="${s.at}" stop-color="${s.color}"${s.opacity === undefined ? "" : ` stop-opacity="${s.opacity}"`}/>`)
    .join("");
}

/** Gradiente CSS em coordenadas do corpo (0..BODY). */
function gradient(id: string, g: Radial | Linear): string {
  if (g.kind === "radial") {
    const t = `translate(${n(g.cx * BODY)} ${n(g.cy * BODY)}) scale(${n(g.rx * BODY)} ${n(g.ry * BODY)})`;
    return `<radialGradient id="${id}" gradientUnits="userSpaceOnUse" cx="0" cy="0" r="1" gradientTransform="${t}">${stops(g.stops)}</radialGradient>`;
  }
  // `linear-gradient(<deg>)`: a linha passa pelo centro na direção do ângulo
  // (0deg = para cima, sentido horário) e tem o comprimento que faz os cantos
  // opostos caírem em 0% e 100%.
  const rad = (g.deg * Math.PI) / 180;
  const dx = Math.sin(rad);
  const dy = -Math.cos(rad);
  const half = (BODY * (Math.abs(dx) + Math.abs(dy))) / 2;
  const c = BODY / 2;
  return `<linearGradient id="${id}" gradientUnits="userSpaceOnUse" x1="${n(c - dx * half)}" y1="${n(c - dy * half)}" x2="${n(c + dx * half)}" y2="${n(c + dy * half)}">${stops(g.stops)}</linearGradient>`;
}

function tile(v: Variant): string {
  const defs: string[] = [];
  const fills = v.layers.map((layer, i) => {
    if (typeof layer === "string") return layer;
    const id = `bg${i}`;
    defs.push(gradient(id, layer));
    return `url(#${id})`;
  });
  const rects = fills.map((f) => `<rect width="${BODY}" height="${BODY}" rx="${RADIUS}" fill="${f}"/>`).join("\n    ");
  const e = n(EDGE / 2);
  const edge = `<rect x="${e}" y="${e}" width="${n(BODY - EDGE)}" height="${n(BODY - EDGE)}" rx="${n(RADIUS - EDGE / 2)}" fill="none" stroke="${v.edge}" stroke-opacity="${v.edgeOpacity}" stroke-width="${n(EDGE)}"/>`;
  if (v.glow) {
    defs.push(
      `<filter id="glow" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="1.6" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge></filter>`,
    );
  }
  const body = markBody(v.mark);
  const mark = v.glow ? `<g filter="url(#glow)">${body}</g>` : body;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${CANVAS}" height="${CANVAS}" viewBox="0 0 ${CANVAS} ${CANVAS}" fill="none">
  <defs>${defs.join("")}</defs>
  <g transform="translate(${INSET} ${INSET})">
    ${rects}
    ${edge}
    <g transform="translate(${PAD} ${PAD}) scale(${n(MARK / 64)})">${mark}</g>
  </g>
</svg>
`;
}

async function run(cmd: string[]): Promise<void> {
  const proc = Bun.spawn(cmd, { cwd: ROOT, stdout: "ignore", stderr: "inherit" });
  if ((await proc.exited) !== 0) throw new Error(`${cmd.join(" ")} falhou`);
}

/** `tauri icon -p <size>` grava `<size>x<size>.png` na pasta de saída. */
async function rasterize(svg: string, size: number, dest: string): Promise<void> {
  const tmp = mkdtempSync(join(tmpdir(), "tyba-icon-variant-"));
  try {
    const input = join(tmp, "in.svg");
    writeFileSync(input, svg);
    await run(["bunx", "tauri", "icon", input, "-o", join(tmp, "out"), "-p", String(size)]);
    renameSync(join(tmp, "out", `${size}x${size}.png`), dest);
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

if (import.meta.main) {
  mkdirSync(join(OUT, "src"), { recursive: true });
  mkdirSync(join(OUT, "thumb"), { recursive: true });
  for (const v of VARIANTS) {
    const svg = tile(v);
    writeFileSync(join(OUT, "src", `${v.id}.svg`), svg);
    await rasterize(svg, CANVAS, join(OUT, `${v.id}.png`));
    // Miniatura: o mesmo SVG recortado no corpo — a margem transparente só
    // existe para o Dock.
    const thumb = svg
      .replace(`width="${CANVAS}" height="${CANVAS}"`, `width="${BODY}" height="${BODY}"`)
      .replace(`viewBox="0 0 ${CANVAS} ${CANVAS}"`, `viewBox="${INSET} ${INSET} ${BODY} ${BODY}"`);
    await rasterize(thumb, THUMB, join(OUT, "thumb", `${v.id}.png`));
    console.log(v.id);
  }
}
