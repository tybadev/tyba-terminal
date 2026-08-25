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

# Token do app, não do design system: o cartão de bloco precisa de um contorno
# que sobreviva ao sunken preto de vários temas, e isso é decisão do TYBA. Entra
# logo depois de `--tyba-sunken` porque é ali que ele é lido no arquivo.
BLOCK_BORDER = """  /* Contorno do cartão de bloco.
     Própria, e não `--tyba-border`: o cartão vive sobre o sunken, que em vários
     temas é preto absoluto, e ali a borda comum rende 1,12:1 — some, e o que
     separa um bloco do outro passa a ser só o espaçamento. Derivada do TEXTO
     para servir a tema claro e escuro sem uma linha por tema. */
  --tyba-block-border: color-mix(in srgb, var(--tyba-text) 28%, transparent);
"""
anchor = re.search(r'^  --tyba-sunken:[^\n]*\n', tyba_body, re.M)
assert anchor, "tyba.css sem --tyba-sunken no :root — revisar port_styles.py"
tyba_body = (
    tyba_body[: anchor.end()] + BLOCK_BORDER + tyba_body[anchor.end() :]
)

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
  --color-tyba-block-border: var(--tyba-block-border);

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
  --animate-tyba-panel-in: tyba-panel-in 360ms cubic-bezier(0.16, 1, 0.3, 1);
  --animate-tyba-node-in: tyba-node-in 440ms cubic-bezier(0.16, 1, 0.3, 1) both;

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
  /*
   * O DOCUMENTO nunca rola. Num app de janela, a janela É a viewport: quem rola
   * é sempre um scroller interno, com dono e limites conhecidos.
   *
   * Sem isto, qualquer filho que transborde o `h-screen` da raiz faz o corpo
   * rolar — e o sintoma não é conteúdo deslizando dentro de um painel, é a
   * interface INTEIRA se movendo, moldura e barra lateral junto.
   *
   * A brecha é de TEMPO, não de layout, e por isso não some ajeitando altura: a
   * lista de blocos só é montada depois que o core confirma o modo prompt (o
   * `blocked` em `App`), e essa resposta chega por IPC. Na aba recém-aberta,
   * entre o primeiro quadro e a resposta, não existe scroller nenhum sob o
   * ponteiro e a roda cai no documento. Quando a lista aparece, o
   * `overscroll-contain` dela passa a conter o gesto e o mesmo movimento passa
   * a funcionar — daí o "só rola dentro da aba depois que eu clico nela".
   */
  overflow: hidden;
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

/* Sem `position: relative; z-index: 10`. Eles existiam para levantar a
   divisa acima do vizinho — necessário enquanto a sombra era desenhada
   para FORA e caía no território dele. Agora ela é `inset`, mora dentro
   do próprio elemento, e ninguém pode cobri-la.
   O que se ganha é poder aplicar a classe em qualquer lugar: com
   `position: relative` ela quebrava todo elemento `absolute` que também
   precisasse de uma divisa — e havia vários. */
.tyba-divide-b {
  box-shadow: var(--tyba-divider-b);
}
.tyba-divide-t {
  box-shadow: var(--tyba-divider-t);
}
.tyba-divide-r {
  box-shadow: var(--tyba-divider-r);
}
.tyba-divide-l {
  box-shadow: var(--tyba-divider-l);
}

/* Fronteira de ROLAGEM, para lista que corre por baixo de um bloco fixo.
   Não é linha, e a diferença não é só de discrição: a linha afirma "aqui
   há uma divisão" o tempo todo, e o fade só existe quando há conteúdo
   escondido acima — ele informa "tem mais coisa aí em cima". Lista curta
   não fade nada, porque não há o que apagar naquela faixa.
   `mask-image`, e não os dois gradientes opacos da técnica clássica de
   sombra de rolagem: aquela precisa da cor do fundo para se auto-esconder,
   e sobre `.tyba-glass` a cor pintaria uma faixa sólida sobre o vidro.
   Efeito colateral aceito: rolando até o fim, o último item fica meio
   apagado. Se incomodar, o conserto é estado de rolagem — não outro CSS. */
.tyba-scroll-fade {
  -webkit-mask-image: linear-gradient(to bottom, #000 calc(100% - 20px), transparent);
  mask-image: linear-gradient(to bottom, #000 calc(100% - 20px), transparent);
}

/* Separação por luz, para o CROMO da janela — header, sidebar, rodapé.
   O `z-index` volta aqui, e só aqui: a parte `cast` é projetada para fora,
   então precisa ficar acima do vizinho. São três containers simples, sem
   filho posicionado, que é o que torna isso seguro — e é justamente por
   não ser seguro em geral que `.tyba-divide-*` não o tem.
   O conteúdo (faixa de abas, painéis) continua na linha: ali a divisa é
   interrompida pela aba ativa, e isso é gesto, não moldura. */
.tyba-lift-b,
.tyba-lift-t,
.tyba-lift-r {
  position: relative;
  z-index: 20;
}
.tyba-lift-b {
  box-shadow:
    inset 0 -1px 0 var(--tyba-lift-edge),
    0 6px 16px -6px var(--tyba-lift-cast);
}
.tyba-lift-t {
  box-shadow:
    inset 0 1px 0 var(--tyba-lift-edge),
    0 -6px 16px -6px var(--tyba-lift-cast);
}
.tyba-lift-r {
  box-shadow:
    inset -1px 0 0 var(--tyba-lift-edge),
    6px 0 16px -6px var(--tyba-lift-cast);
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

/* Painel lateral Agentes: entra com fade + slide, sai só com fade. O unmount
   real fica a cargo do usePresence; aqui só o quadro visual. */
@keyframes tyba-panel-in {
  from {
    opacity: 0;
    transform: translateX(20px) scale(0.985);
  }
  to {
    opacity: 1;
    transform: translateX(0) scale(1);
  }
}

.tyba-panel-exit {
  animation: tyba-panel-out 150ms ease forwards;
}

@keyframes tyba-panel-out {
  from {
    opacity: 1;
    transform: translateX(0);
  }
  to {
    opacity: 0;
    transform: translateX(12px);
  }
}

@keyframes tyba-node-in {
  from {
    opacity: 0;
    transform: translateY(9px);
    filter: blur(1px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
    filter: blur(0);
  }
}

@media (prefers-reduced-motion: reduce) {
  .tyba-panel-exit {
    animation: none;
  }
}

/* xterm ocupa o container inteiro */
.xterm {
  height: 100%;
}

/* Preview de markdown do painel Arquivos: tipografia proporcional, chips de
   código inline e blocos com fundo; herda os tokens de tema (claro/escuro). */
.files-markdown {
  padding: 16px 20px;
  font-family: var(--tyba-font-sans);
  font-size: 0.8125rem;
  line-height: 1.7;
  color: var(--tyba-text);
  overflow-wrap: anywhere;
}
.files-markdown > :first-child {
  margin-top: 0;
}
.files-markdown > :last-child {
  margin-bottom: 0;
}
.files-markdown h1,
.files-markdown h2,
.files-markdown h3,
.files-markdown h4,
.files-markdown h5,
.files-markdown h6 {
  font-family: var(--tyba-font-sans);
  font-weight: 600;
  line-height: 1.3;
  color: var(--tyba-text);
  margin: 1.6em 0 0.6em;
}
.files-markdown h1 {
  font-size: 1.6rem;
  padding-bottom: 0.3em;
  border-bottom: 1px solid var(--tyba-border);
}
.files-markdown h2 {
  font-size: 1.3rem;
  padding-bottom: 0.25em;
  border-bottom: 1px solid var(--tyba-border);
}
.files-markdown h3 {
  font-size: 1.1rem;
}
.files-markdown h4 {
  font-size: 0.95rem;
}
.files-markdown h5,
.files-markdown h6 {
  font-size: 0.85rem;
  color: var(--tyba-text-muted);
}
.files-markdown p {
  margin: 0.7em 0;
}
.files-markdown a {
  color: var(--tyba-green);
  text-decoration: underline;
  text-underline-offset: 2px;
}
.files-markdown strong {
  font-weight: 600;
  color: var(--tyba-text);
}
.files-markdown em {
  font-style: italic;
}
.files-markdown ul,
.files-markdown ol {
  margin: 0.7em 0;
  padding-left: 1.5em;
}
.files-markdown li {
  margin: 0.25em 0;
}
.files-markdown li > ul,
.files-markdown li > ol {
  margin: 0.25em 0;
}
.files-markdown blockquote {
  margin: 0.9em 0;
  padding: 0.2em 0 0.2em 1em;
  border-left: 3px solid var(--tyba-border-strong);
  color: var(--tyba-text-muted);
}
.files-markdown hr {
  border: 0;
  border-top: 1px solid var(--tyba-border);
  margin: 1.5em 0;
}
.files-markdown img {
  max-width: 100%;
  border-radius: 6px;
}
.files-md-inline {
  font-family: var(--tyba-font-mono);
  font-size: 0.85em;
  padding: 0.12em 0.4em;
  border-radius: 4px;
  background: color-mix(in srgb, var(--tyba-text) 8%, transparent);
  border: 1px solid var(--tyba-border);
  white-space: break-spaces;
}
.files-md-pre {
  margin: 0.9em 0;
  padding: 12px 14px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--tyba-text) 6%, transparent);
  border: 1px solid var(--tyba-border);
  overflow-x: auto;
}
.files-md-pre code {
  font-family: var(--tyba-font-mono);
  font-size: 0.8125rem;
  line-height: 1.5;
  color: var(--tyba-text);
  background: none;
  border: 0;
  padding: 0;
  white-space: pre;
}
.files-markdown table {
  display: block;
  border-collapse: collapse;
  margin: 1em 0;
  font-size: 0.8125rem;
  overflow-x: auto;
}
.files-markdown th,
.files-markdown td {
  border: 1px solid var(--tyba-border);
  padding: 0.4em 0.7em;
  text-align: left;
}
.files-markdown th {
  background: color-mix(in srgb, var(--tyba-text) 8%, transparent);
  font-weight: 600;
}
.files-markdown tbody tr:nth-child(even) {
  background: color-mix(in srgb, var(--tyba-text) 3%, transparent);
}
'''

out = header + tyba_body.rstrip() + "\n\n" + themes.rstrip() + tail


def declarations(css: str) -> set[str]:
    """O que o arquivo DEFINE: custom properties e seletores de topo.

    É o suficiente para pegar perda. Não é um parser de CSS e não precisa ser —
    a pergunta é "sumiu alguma coisa?", não "o valor mudou?".
    """
    css = re.sub(r'/\*.*?\*/', '', css, flags=re.S)
    props = {m.group(1) for m in re.finditer(r'(--[\w-]+)\s*:', css)}
    # O prelúdio de cada bloco é o texto entre o `}` anterior e o `{` — daí
    # `[^{}]*`, que não atravessa bloco. Vírgula separa a lista de seletores
    # (`html,\nbody,\n#root {`), e cada parte entra sozinha.
    heads = set()
    for m in re.finditer(r'([^{}]*)\{', css):
        # O que vem antes do último `;` é at-rule sem bloco (`@import …;`) ou
        # sobra de declaração — não faz parte do seletor que abre aqui.
        head = m.group(1).rsplit(';', 1)[-1]
        for part in head.split(','):
            part = ' '.join(part.split())
            if part:
                heads.add(part)
    return props | heads


# > [!warning] Sem esta guarda, rodar o script apaga o que ele não gera.
#
# O `styles.css` diz no cabeçalho que é montado por aqui, e é: mas por meses o
# CSS do app foi escrito direto no arquivo, sem espelhar no `tail`. Em 15/08/2026
# uma remontagem levou junto 195 linhas em uso — `--tyba-block-border` (o
# contorno de todo cartão de bloco), as animações do painel Agentes e o
# `.files-markdown` inteiro. Nenhum gate pegaria: classe CSS ausente não quebra
# typecheck nem teste, e o estrago só aparece na tela, depois.
#
# A guarda compara o que o arquivo definia com o que a remontagem define. Some
# alguma coisa, o script para e diz o que fazer. Remoção deliberada passa com
# `--force`.
if APP.exists() and '--force' not in sys.argv:
    perdidas = sorted(declarations(APP.read_text()) - declarations(out))
    if perdidas:
        print(
            f"erro: a remontagem perderia {len(perdidas)} declaração(ões) que só "
            f"existem em {APP.name}:",
            file=sys.stderr,
        )
        for nome in perdidas:
            print(f"  {nome}", file=sys.stderr)
        print(
            "\nCSS do app mora no `tail` deste script, não no arquivo gerado — "
            "mova para lá e rode de novo.\nSe a remoção é intencional, "
            "`--force`.",
            file=sys.stderr,
        )
        sys.exit(1)

APP.write_text(out)
print(f"ok {APP} ({len(out.splitlines())} linhas)")
