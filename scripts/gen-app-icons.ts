/**
 * Gera os ícones do app a partir da marca (`src-tauri/icons/art/tyba-mark.svg`,
 * cópia de `tyba-design-system/ds-bundle/assets/tyba-mark.svg`).
 *
 * Roda à mão, não no build — os arquivos gerados são commitados:
 *
 *   bun scripts/gen-app-icons.ts
 *
 * Saídas:
 *
 * - `icons/art/app-icon-{dark,light}.svg` — o ladrilho 1024×1024 de cada tema:
 *   a marca colorida sobre #050505 (BLACKOUT) ou sobre #fafbff (Aurora).
 * - `icons/*` (icns, ico, PNGs, Square*Logo, android, ios) — o conjunto padrão
 *   do `tauri icon`, a partir do ladrilho **dark**. É o que Windows, Linux e o
 *   macOS anterior ao 26 usam: nenhum deles troca o ícone pelo tema do sistema.
 * - `icons/light/*.png` — o conjunto light em PNG, para quem empacotar um tema
 *   claro no Linux (o `.desktop` não troca ícone por aparência).
 * - `icons/Tyba.icon/` — documento do Icon Composer (fonte editável) com o
 *   fundo especializado por aparência: #fafbff no claro, #050505 no escuro.
 * - `icons/Assets.car` — o `Tyba.icon` compilado pelo `actool` (Xcode ≥ 26,
 *   só no macOS). É ele que o `tauri.macos.conf.json` lista: o bundler copia o
 *   `.car` para `Resources/` e grava `CFBundleIconName`, e no macOS 26+ o
 *   ícone acompanha claro/escuro/tingido. O `icon.icns` (dark) continua como
 *   `CFBundleIconFile` para o macOS < 26.
 *
 *   Compilado aqui, e não pelo bundler a partir do `.icon`, por dois motivos:
 *   o runner de CI não precisa de Xcode 26 para o release sair com o ícone
 *   adaptável, e o `actool` chamado de dentro do `tauri build` falhou de forma
 *   intermitente ("Bad file descriptor" do `ibtoold`) na máquina do dono.
 *   Se o `actool` falhar aqui também: `pkill -f ibtoold` e rode de novo.
 */
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const ROOT = resolve(import.meta.dir, "..");
const ICONS = join(ROOT, "src-tauri", "icons");
const ART = join(ICONS, "art");

/** Constantes da marca (brand.md): BLACKOUT e Aurora. */
const THEMES = {
  dark: { bg: "#050505", edge: "rgba(255,255,255,0.10)" },
  light: { bg: "#fafbff", edge: "rgba(5,5,5,0.08)" },
} as const;

/**
 * Grade de ícone do macOS em 1024: o ladrilho ocupa 824 com 100 de margem
 * transparente (senão o Dock desenha um quadrado sangrando) e raio ~185.
 */
const TILE = { inset: 100, size: 824, radius: 185 };
/** Tamanho da marca dentro do ladrilho. */
const MARK_SIZE = 540;

// O comentário do SVG da marca tem `--`, que é XML inválido: o `actool` recusa.
const mark = readFileSync(join(ART, "tyba-mark.svg"), "utf8")
  .replace(/<!--[\s\S]*?-->\s*/g, "")
  .trim();
const inner = mark.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "");

function tile(theme: keyof typeof THEMES): string {
  const { bg, edge } = THEMES[theme];
  const offset = (1024 - MARK_SIZE) / 2;
  const scale = MARK_SIZE / 64;
  const { inset, size, radius } = TILE;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <rect x="${inset}" y="${inset}" width="${size}" height="${size}" rx="${radius}" fill="${bg}"/>
  <rect x="${inset + 1}" y="${inset + 1}" width="${size - 2}" height="${size - 2}" rx="${radius - 1}" fill="none" stroke="${edge}" stroke-width="2"/>
  <g transform="translate(${offset} ${offset}) scale(${scale})">${inner}</g>
</svg>
`;
}

async function run(cmd: string[]): Promise<void> {
  const proc = Bun.spawn(cmd, { cwd: ROOT, stdout: "inherit", stderr: "inherit" });
  if ((await proc.exited) !== 0) throw new Error(`${cmd.join(" ")} falhou`);
}

/** Cor em `srgb:r,g,b,a` (0–1), o formato do `icon.json`. */
function srgb(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  const c = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => (v / 255).toFixed(5));
  return `srgb:${c.join(",")},1.00000`;
}

function iconComposer(): void {
  const dir = join(ICONS, "Tyba.icon");
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(join(dir, "Assets"), { recursive: true });
  // Camada única: a marca. No Icon Composer o canvas de 1024 É o ladrilho (o
  // sistema recorta e põe a margem), então a marca escala por 1024/824 para ter
  // o mesmo peso do ícone legado.
  const size = Math.round((MARK_SIZE * 1024) / TILE.size);
  const layer = mark.replace("<svg ", `<svg width="${size}" height="${size}" `);
  writeFileSync(join(dir, "Assets", "tyba-mark.svg"), layer);
  const doc = {
    "fill-specializations": [
      { value: { solid: srgb(THEMES.light.bg) } },
      { appearance: "dark", value: { solid: srgb(THEMES.dark.bg) } },
    ],
    groups: [
      {
        layers: [{ "image-name": "tyba-mark.svg", name: "tyba-mark" }],
        shadow: { kind: "neutral", opacity: 0.5 },
        translucency: { enabled: false, value: 0.5 },
      },
    ],
    "supported-platforms": { squares: ["macOS"] },
  };
  writeFileSync(join(dir, "icon.json"), `${JSON.stringify(doc, null, 2)}\n`);
}

const dark = join(ART, "app-icon-dark.svg");
const light = join(ART, "app-icon-light.svg");
writeFileSync(dark, tile("dark"));
writeFileSync(light, tile("light"));

/** `Tyba.icon` → `Assets.car`, com os mesmos argumentos que o bundler do Tauri usa. */
async function compileAssetsCar(): Promise<void> {
  if (process.platform !== "darwin") {
    console.warn("Assets.car não regenerado: o actool só existe no macOS.");
    return;
  }
  const tmp = mkdtempSync(join(tmpdir(), "tyba-icon-"));
  try {
    cpSync(join(ICONS, "Tyba.icon"), join(tmp, "Tyba.icon"), { recursive: true });
    mkdirSync(join(tmp, "out"));
    await run([
      "xcrun", "actool", join(tmp, "Tyba.icon"),
      "--compile", join(tmp, "out"),
      "--output-format", "human-readable-text", "--notices", "--warnings",
      "--output-partial-info-plist", join(tmp, "partial.plist"),
      "--app-icon", "Tyba", "--include-all-app-icons",
      "--enable-on-demand-resources", "NO", "--development-region", "en",
      "--target-device", "mac", "--minimum-deployment-target", "26.0", "--platform", "macosx",
    ]);
    cpSync(join(tmp, "out", "Assets.car"), join(ICONS, "Assets.car"));
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

await run(["bunx", "tauri", "icon", dark, "-o", ICONS, "--ios-color", THEMES.dark.bg]);
rmSync(join(ICONS, "light"), { recursive: true, force: true });
await run([
  "bunx", "tauri", "icon", light, "-o", join(ICONS, "light"),
  "-p", "32", "-p", "64", "-p", "128", "-p", "256", "-p", "512",
]);
iconComposer();
await compileAssetsCar();
