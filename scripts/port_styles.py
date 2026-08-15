#!/usr/bin/env python3
"""Monta tyba-terminal/src/styles.css a partir dos tokens do design system."""
import pathlib, re, sys

DS = pathlib.Path('/Users/guilherme/swell-system/tyba-design-system/ds-bundle')
APP = pathlib.Path(__file__).resolve().parent.parent / 'src' / 'styles.css'

tyba = (DS / 'tokens' / 'tyba.css').read_text()
themes = (DS / 'tokens' / 'themes.css').read_text()

# Corta o cabeçalho do tyba.css (a partir de ":root {")
tyba_body = tyba[tyba.index(':root {'):]

# Injeta a ponte shadcn + raio no fim do bloco :root (antes do "}" que
# precede o bloco [data-font='grotesk'])
BRIDGE = """
  /* ---------- Ponte semântica shadcn/ui → tokens TYBA ----------
     Os componentes de src/components/ui usam estes nomes; como
     apontam para --tyba-*, trocam de tema junto. */
  --background: var(--tyba-bg);
  --foreground: var(--tyba-text);
  --card: var(--tyba-raised);
  --card-foreground: var(--tyba-text);
  --popover: var(--tyba-overlay);
  --popover-foreground: var(--tyba-text);
  --primary: var(--tyba-primary);
  --primary-foreground: var(--tyba-text-inverse);
  --secondary: var(--tyba-raised);
  --secondary-foreground: var(--tyba-text);
  --muted: var(--tyba-raised);
  --muted-foreground: var(--tyba-text-muted);
  --accent: var(--tyba-raised);
  --accent-foreground: var(--tyba-text);
  --destructive: var(--tyba-red);
  --destructive-foreground: #ffffff;
  --border: var(--tyba-border);
  --input: var(--tyba-border-strong);
  --ring: var(--tyba-green);
  /* raios contidos: é terminal, não site */
  --radius: 6px;
}
"""
marker = "}\n\n/* Fonte da UI alternativa"
assert marker in tyba_body, "estrutura do tyba.css mudou — revisar port_styles.py"
tyba_body = tyba_body.replace(marker, BRIDGE + "\n/* Fonte da UI alternativa", 1)

header = '''@import "tailwindcss";

/* ============================================================
   Identidade TYBA — tokens v3 do design system (BLACKOUT).
   Fonte da verdade: repo tyba-design-system (ds-bundle/tokens/),
   espelhado no projeto "TYBA Design System" do claude.ai/design.
   Este arquivo é MONTADO a partir de tyba.css + themes.css pelo
   script scripts/port_styles.py — ao mudar tokens lá, remontar aqui.
   ============================================================ */

'''

tail = '''

/* dark: do Tailwind segue o esquema do tema — todo tema claro
   termina em "light" (light, solarized-light, monokai-light...) */
@custom-variant dark (&:is(:root:not([data-theme$="light"]) *, :root:not([data-theme$="light"])));

/* Vocabulário Tailwind: bg-tyba-raised, text-tyba-green, bg-background… */
@theme inline {
  --color-tyba-bg: var(--tyba-bg);
  --color-tyba-surface: var(--tyba-surface);
  --color-tyba-raised: var(--tyba-raised);
  --color-tyba-overlay: var(--tyba-overlay);
  --color-tyba-sunken: var(--tyba-sunken);
  --color-tyba-border: var(--tyba-border);
  --color-tyba-border-strong: var(--tyba-border-strong);

  --color-tyba-text: var(--tyba-text);
  --color-tyba-text-muted: var(--tyba-text-muted);
  --color-tyba-text-faint: var(--tyba-text-faint);
  --color-tyba-text-inverse: var(--tyba-text-inverse);

  --color-tyba-green: var(--tyba-green);
  --color-tyba-lime: var(--tyba-lime);
  --color-tyba-amber: var(--tyba-amber);
  --color-tyba-magenta: var(--tyba-magenta);
  --color-tyba-violet: var(--tyba-violet);
  --color-tyba-blue: var(--tyba-blue);
  --color-tyba-cyan: var(--tyba-cyan);
  --color-tyba-red: var(--tyba-red);

  --color-tyba-green-tint: var(--tyba-green-tint);
  --color-tyba-amber-tint: var(--tyba-amber-tint);
  --color-tyba-magenta-tint: var(--tyba-magenta-tint);
  --color-tyba-violet-tint: var(--tyba-violet-tint);
  --color-tyba-blue-tint: var(--tyba-blue-tint);
  --color-tyba-cyan-tint: var(--tyba-cyan-tint);
  --color-tyba-red-tint: var(--tyba-red-tint);
  --color-tyba-neutral-tint: var(--tyba-neutral-tint);

  --animate-tyba-pop-in: tyba-pop-in 180ms cubic-bezier(0.16, 1, 0.3, 1);

  /* shadcn */
  --color-background: var(--background);
  --color-foreground: var(--foreground);
  --color-card: var(--card);
  --color-card-foreground: var(--card-foreground);
  --color-popover: var(--popover);
  --color-popover-foreground: var(--popover-foreground);
  --color-primary: var(--primary);
  --color-primary-foreground: var(--primary-foreground);
  --color-secondary: var(--secondary);
  --color-secondary-foreground: var(--secondary-foreground);
  --color-muted: var(--muted);
  --color-muted-foreground: var(--muted-foreground);
  --color-accent: var(--accent);
  --color-accent-foreground: var(--accent-foreground);
  --color-destructive: var(--destructive);
  --color-destructive-foreground: var(--destructive-foreground);
  --color-border: var(--border);
  --color-input: var(--input);
  --color-ring: var(--ring);

  --radius-sm: calc(var(--radius) - 4px);
  --radius-md: calc(var(--radius) - 2px);
  --radius-lg: var(--radius);
  --radius-xl: calc(var(--radius) + 4px);

  --font-sans: "Space Grotesk", ui-sans-serif, system-ui, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, "SF Mono", Menlo, monospace;
  --font-ui: var(--tyba-font-ui);
}

html,
body,
#root {
  height: 100%;
  margin: 0;
  background: var(--tyba-bg);
  color: var(--tyba-text);
  font-family: var(--tyba-font-ui);
  -webkit-font-smoothing: antialiased;
}

::selection {
  background: var(--tyba-selection);
}

/* Ligatures ligadas por padrão (=> -> === !=) — é terminal, ligatures
   são parte da estética. Desligue pontualmente com .tyba-no-ligatures.
   (xterm não herda isso: exige @xterm/addon-ligatures, pendente) */
code,
pre,
kbd,
samp {
  font-family: var(--tyba-font-mono);
  font-variant-ligatures: contextual;
  font-feature-settings: "calt" 1, "liga" 1;
}

.tyba-no-ligatures {
  font-variant-ligatures: none;
  font-feature-settings: "calt" 0, "liga" 0;
}

/* Scrollbar discreta, estilo app desktop (vale também pro xterm) */
::-webkit-scrollbar {
  width: 10px;
  height: 10px;
}
::-webkit-scrollbar-track {
  background: transparent;
}
::-webkit-scrollbar-thumb {
  background: var(--tyba-border-strong);
  border-radius: 5px;
  border: 2px solid transparent;
  background-clip: content-box;
}
::-webkit-scrollbar-thumb:hover {
  background-color: var(--tyba-text-faint);
}

/* ---------- Utilitárias de marca ---------- */

/* Texto com o gradiente-assinatura (momentos de marca apenas) */
.tyba-gradient-text {
  background: var(--tyba-gradient);
  background-clip: text;
  -webkit-background-clip: text;
  color: transparent;
}

/* Borda com gradiente (cards de destaque) — dois fundos, não ::before */
.tyba-gradient-border {
  border: 1px solid transparent;
  border-radius: var(--tyba-radius-lg);
  background:
    var(--tyba-sheen) padding-box,
    var(--tyba-gradient) border-box;
  box-shadow: var(--tyba-edge);
}

/* Superfície que pega luz (BLACKOUT): card de vidro escuro — verniz
   vertical + aresta iluminada no topo. É O padrão para cards no dark:
   a separação vem da luz, não de um cinza mais claro. */
.tyba-lit {
  background: var(--tyba-sheen);
  border: 1px solid var(--tyba-border);
  border-radius: var(--tyba-radius-lg);
  box-shadow: var(--tyba-edge), var(--tyba-shadow-md);
}

/* Label de seção em caixa alta (estilo do brand board) */
.tyba-label {
  font-size: var(--tyba-text-xs);
  font-weight: 500;
  letter-spacing: var(--tyba-tracking-wide);
  text-transform: uppercase;
  color: var(--tyba-text-faint);
}

/* Anel de foco padrão para elementos interativos */
.tyba-focusable:focus-visible {
  outline: none;
  box-shadow: var(--tyba-focus-ring);
}

.tyba-divide-b,
.tyba-divide-t,
.tyba-divide-r {
  position: relative;
  z-index: 10;
}
.tyba-divide-b {
  box-shadow: var(--tyba-divider-b);
}
.tyba-divide-t {
  box-shadow: var(--tyba-divider-t);
}
.tyba-divide-r {
  box-shadow: var(--tyba-divider-r);
}

/* ---------- Luz ---------- */

/* Canvas com aurora: respiro de cor no topo do app */
.tyba-aurora {
  background-image: var(--tyba-aurora);
  background-color: var(--tyba-bg);
}

/* Vidro sobre o void (sidebar, painéis flutuantes) */
.tyba-glass {
  background: var(--tyba-glass-bg);
  backdrop-filter: blur(var(--tyba-glass-blur));
  -webkit-backdrop-filter: blur(var(--tyba-glass-blur));
}

/* Linha viva: filete de luz que marca o que está ativo.
   Estática = selecionado; --live (animada) = executando agora. */
.tyba-flow-line {
  height: 1px;
  border-radius: 1px;
  background: var(--tyba-flow-gradient);
}
.tyba-flow-line--live {
  background-size: 200% 100%;
  animation: tyba-flow 3s linear infinite;
}
/* Modificador de espessura, e não uma utilitária do Tailwind: `.tyba-flow-line`
   fixa `height` fora de camada, e utilitária de mesma especificidade em @layer
   perde a disputa — o fio continuaria com 1px sem ninguém entender por quê. */
.tyba-flow-line--thick {
  height: 2px;
  border-radius: 2px;
}
@keyframes tyba-flow {
  from { background-position: 0% 0; }
  to { background-position: -200% 0; }
}
@media (prefers-reduced-motion: reduce) {
  .tyba-flow-line--live { animation: none; }
}

@keyframes tyba-pop-in {
  from {
    opacity: 0;
    transform: scale(0.95);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}

/* xterm ocupa o container inteiro */
.xterm {
  height: 100%;
}
'''

out = header + tyba_body.rstrip() + "\n\n" + themes.rstrip() + tail
APP.write_text(out)
print(f"ok {APP} ({len(out.splitlines())} linhas)")
