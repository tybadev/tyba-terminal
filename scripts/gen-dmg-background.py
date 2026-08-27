#!/usr/bin/env python3
"""Gera o fundo da janela de instalação do DMG, nos tokens do TYBA.

Roda à mão, não no build: o PNG é artefato versionado, e build que gera
imagem quebra sem a fonte instalada.

    python3 scripts/gen-dmg-background.py

O desenho segue a regra que os próprios tokens declaram — *"preto de verdade:
as camadas quase não clareiam; quem separa é a LUZ, não o cinza"*. Por isso não
há caixa, moldura nem painel: o fundo é o `--tyba-bg` caindo para o
`--tyba-sunken`, que é exatamente a estratificação do app (fundo → área de
terminal).

No lugar da seta genérica de DMG vão TRÊS chevrons `›` — o mesmo glifo que o
TYBA desenha antes da linha de comando. Ele aponta para os Aplicativos e é a
marca do produto ao mesmo tempo; o brilho cresce da esquerda para a direita
para o olho ser puxado ao destino.
"""

import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

RAIZ = Path(__file__).resolve().parent.parent

# As medidas vêm da CONFIG, não repetidas aqui.
#
# O fundo tem de casar com onde o bundler larga os dois ícones, e enquanto os
# dois lados guardassem sua própria cópia dos números, mudar um e esquecer o
# outro sairia como desalinhamento silencioso: a imagem continua bonita, os
# chevrons é que deixam de apontar para a pasta.
#
# A janela é medida em PONTOS, e o Finder desenha o fundo no tamanho natural,
# ancorado no topo-esquerdo — ele NÃO escala. Uma imagem 2x apareceria cortada
# no quarto superior esquerdo, então a imagem sai do tamanho da janela.
_DMG = json.loads((RAIZ / "src-tauri/tauri.conf.json").read_text())["bundle"]["macOS"]["dmg"]
LARGURA, ALTURA = _DMG["windowSize"]["width"], _DMG["windowSize"]["height"]

# Tokens, copiados de `src/styles.css` (tema BLACKOUT, o canônico).
SURFACE = (10, 10, 11)  # --tyba-surface
SUNKEN = (0, 0, 0)  # --tyba-sunken
# `--tyba-border`: rgba(255,255,255,0.07) resolvido sobre o preto. O comentário
# do token é literal — "borda = luz, não cinza" —, e é a única régua do desenho.
LUZ = (18, 18, 18)
# O chão claro sob os ícones. Claro o bastante para o preto do Finder render
# contraste de leitura, e neutro para não brigar com o azul da pasta Aplicativos.
# A zona clara: os tokens do tema light do TYBA, não um cinza inventado.
CLARO_BG = (245, 245, 247)      # --tyba-bg  (light)
CLARO_SUNKEN = (236, 236, 240)  # --tyba-sunken (light)
CLARO_TEXTO = (95, 96, 102)     # --tyba-text-faint, que no claro é o texto discreto
CORTE_Y = 238                   # logo abaixo do ícone: a arte no escuro, o rótulo no claro

# A barra de título come 31 pt do alto, e `windowSize` mede a janela INTEIRA.
#
# Medido na captura da janela real: 720 px de janela a 2x = 360 pt, e o conteúdo
# começa em 62 px = 31 pt. Então a faixa útil do fundo é 31 pt mais curta do que
# a imagem, e tudo que for desenhado abaixo disso NUNCA é visto — sem erro, sem
# aviso, só sumindo. Foi assim que a legenda quase encostou numa borda que não
# existe na imagem.
ALTURA_TITULO = 31
VISIVEL = ALTURA - ALTURA_TITULO
VERDE = (124, 197, 68)  # --tyba-green: terminal, sucesso, "ativo"
MUTED = (159, 160, 166)  # --tyba-text-muted

# Onde o bundler larga os dois ícones. O chevron mora no meio deles.
APP_X, ICONE_Y = _DMG["appPosition"]["x"], _DMG["appPosition"]["y"]
PASTA_X = _DMG["applicationFolderPosition"]["x"]

FONTE = "/Users/guilherme/Library/Fonts/JetBrainsMono-{}.ttf"


def desenha() -> Image.Image:
    img = Image.new("RGB", (LARGURA, ALTURA), SURFACE)
    d = ImageDraw.Draw(img)

    # `--tyba-surface` caindo para `--tyba-sunken`: a mesma queda que separa o
    # painel da área de terminal no app. Dez níveis em quatrocentos pixels —
    # perceptível como superfície, longe de virar cinza.
    for y in range(ALTURA):
        t = (y / (ALTURA - 1)) ** 1.6
        d.line(
            [(0, y), (LARGURA, y)],
            fill=tuple(round(SURFACE[c] + (SUNKEN[c] - SURFACE[c]) * t) for c in range(3)),
        )

    # ONDE O ESCURO ACABA — e isto não é escolha estética, é legibilidade.
    #
    # A janela de DMG com imagem de fundo desenha os rótulos dos ícones em PRETO
    # mesmo com o sistema em modo escuro: conferido por captura da janela real,
    # luminância 1,0 de texto sobre 5,0 de fundo, num Mac com
    # `AppleInterfaceStyle = Dark`. Fundo escuro apaga "Tyba" e "Applications"
    # SEMPRE, e não é variação de máquina que dê para aceitar.
    #
    # A primeira tentativa foi espalhar luz sob os ícones, e virou névoa: comeu
    # a metade de baixo dos dois e matou o preto que é a marca. A saída é a
    # oposta — não difundir, DIVIDIR. O TYBA tem os dois temas; a janela mostra
    # os dois, e a linha onde eles se encontram é a mesma hairline de luz que
    # separa um cartão do fundo no app.
    #
    # O corte cai logo abaixo dos ícones: a arte fica no escuro, o rótulo nasce
    # no claro. Nada é empurrado para caber — é onde o Finder já os desenha.
    claro = Image.new("RGB", (LARGURA, ALTURA - CORTE_Y), CLARO_BG)
    dc = ImageDraw.Draw(claro)
    for y in range(claro.height):
        t = (y / max(1, claro.height - 1)) ** 1.4
        dc.line(
            [(0, y), (LARGURA, y)],
            fill=tuple(round(CLARO_BG[c] + (CLARO_SUNKEN[c] - CLARO_BG[c]) * t) for c in range(3)),
        )
    img.paste(claro, (0, CORTE_Y))
    d = ImageDraw.Draw(img)

    # A luz na junta. Some nas pontas para ser uma linha de luz, não uma moldura.
    for x in range(LARGURA):
        h = min(1.0, min(x, LARGURA - 1 - x) / (LARGURA * 0.30))
        d.point((x, CORTE_Y), fill=tuple(round(255 * h) for _ in range(3)))

    # Os três chevrons. O da direita é o mais forte: o olho anda com o brilho, e
    # ele tem de andar para os Aplicativos.
    fonte_chevron = ImageFont.truetype(FONTE.format("Bold"), 30)
    meio = (APP_X + PASTA_X) / 2
    for i, alfa in enumerate((0.22, 0.5, 1.0)):
        d.text(
            (meio + (i - 1) * 17, ICONE_Y),
            "›",
            font=fonte_chevron,
            fill=tuple(round(SUNKEN[c] + (VERDE[c] - SUNKEN[c]) * alfa) for c in range(3)),
            anchor="mm",
        )

    # A instrução ganha lugar por FAZER algo — dizer o gesto —, não por decorar.
    # Fica abaixo do rótulo dos dois ícones, que o Finder desenha por volta de
    # y=250.
    # Derivada da faixa VISÍVEL, não da altura da imagem: mudar a altura da
    # janela na config não pode deixar a legenda pendurada, nem empurrá-la para
    # baixo do corte da barra de título.
    d.text(
        (LARGURA / 2, CORTE_Y + (VISIVEL - CORTE_Y) * 0.68),
        "Arraste o Tyba para Aplicativos",
        font=ImageFont.truetype(FONTE.format("Regular"), 11),
        fill=CLARO_TEXTO,
        anchor="mm",
    )
    return img


if __name__ == "__main__":
    alvo = RAIZ / "src-tauri" / _DMG["background"]
    desenha().save(alvo)
    print(f"{alvo.relative_to(RAIZ)} · {LARGURA}x{ALTURA}")
