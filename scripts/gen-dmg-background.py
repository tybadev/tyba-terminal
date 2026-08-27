#!/usr/bin/env python3
"""Gera o fundo da janela de instalação do DMG, nos tokens do TYBA.

Roda à mão, não no build: o PNG é artefato versionado, e build que gera
imagem quebra sem a fonte instalada.

    python3 scripts/gen-dmg-background.py

O desenho segue a regra que os próprios tokens declaram — *"preto de verdade:
as camadas quase não clareiam; quem separa é a LUZ, não o cinza"*. O fundo é o
`--tyba-surface` caindo para o `--tyba-sunken`, que é exatamente a estratificação
do app (fundo → área de terminal), e a janela é preta de ponta a ponta.

A ÚNICA coisa clara são duas pastilhas, uma atrás de cada rótulo. Elas existem
porque o Finder desenha "Tyba" e "Applications" em PRETO mesmo com o sistema em
modo escuro, e isso não é configurável. Ficam pastilha, e não faixa, porque
qualquer região clara contínua é lida como fundo faltando — testado duas vezes,
com faixa até a base e com rodapé fino, e as duas foram reprovadas assim.

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
# O claro da pastilha. Claro o bastante para o preto do Finder render contraste
# de leitura, e neutro para não brigar com o azul da pasta Aplicativos. Vem do
# tema light do TYBA, não é um cinza inventado.
CLARO_BG = (245, 245, 247)      # --tyba-bg  (light)
# A barra de título come 32 pt do alto, e `windowSize` mede a janela INTEIRA.
#
# Medido na janela real por varredura de luminância: a barra vai de 0 a 32 pt, e
# o corte claro/escuro da imagem apareceu exatamente 238 pt abaixo do topo do
# CONTEÚDO — confirmando que o Finder ancora o fundo no topo do conteúdo, 1:1,
# sem escalar. Logo a faixa útil é 328 pt, e tudo desenhado abaixo disso NUNCA é
# visto: sem erro, sem aviso, só sumindo.
#
# ARMADILHA, e ela já cobrou: não basta CALCULAR a faixa visível, é preciso
# USÁ-LA. A versão anterior definia este mesmo valor e depois desenhava tudo com
# `ALTURA`; só a legenda consultava `VISIVEL`. Saíram 32 px de rodapé cortados
# fora — 26% dele —, e a janela lia como fundo que não tinha carregado.
ALTURA_TITULO = 32
VISIVEL = ALTURA - ALTURA_TITULO

# O claro existe para UMA coisa: os dois rótulos que o Finder desenha.
#
# `ROTULO_Y` é medido, não escolhido — é onde o Finder larga "Tyba" e
# "Applications". A legenda logo abaixo é texto NOSSO, desenhado aqui, então vive
# no escuro em `--tyba-text-muted`; só estes dois rótulos são pretos por
# imposição do Finder.
#
# POR QUE PASTILHA E NÃO FAIXA, e isto custou duas rodadas para aprender:
# qualquer região clara contínua — indo até a base da janela ou como rodapé fino
# de 50 pt — é lida como "o fundo não chegou aqui". Não é questão de tamanho: é
# de haver uma área onde o preto do produto some. A pastilha resolve porque a luz
# vira forma, com contorno próprio, e o resto da janela permanece preto.
ROTULO_Y = 254
CHIP_ALTURA = 24
CHIP_PADDING = 12  # metade da altura: a pastilha abraça o nome, não uma caixa em volta dele

# As larguras dos rótulos são MEDIDAS na janela real, varrendo os pixels pretos
# que o Finder desenhou — não estimadas por métrica de fonte, que erraria a fonte
# do sistema.
#
# E o rótulo da pasta NÃO é localizado, ao contrário do que parece: `/Applications`
# tem `.localized` e por isso a pasta real aparece como "Aplicativos", mas dentro
# do DMG é um symlink comum chamado `Applications`, e o Finder mostra o nome
# literal. Conferido num sistema em `pt-BR` — a janela diz "Applications". Logo os
# dois rótulos são fixos e a pastilha pode colar neles.
#
# O que MUDA estas medidas é renomear o app (`productName`) ou trocar o tamanho de
# ícone no `.DS_Store`. Nesse caso, remedir na janela em vez de estimar.
ROTULO_LARGURA_APP = 34
ROTULO_LARGURA_PASTA = 88
CHIP_LARGURA_APP = ROTULO_LARGURA_APP + 2 * CHIP_PADDING
CHIP_LARGURA_PASTA = ROTULO_LARGURA_PASTA + 2 * CHIP_PADDING
# A borda não é moldura: é a hairline de luz do token, só que vista de dentro do
# claro, onde ela precisa escurecer em vez de clarear para continuar sendo borda.
CHIP_BORDA = (52, 52, 54)
VERDE = (124, 197, 68)  # --tyba-green: terminal, sucesso, "ativo"
MUTED = (159, 160, 166)  # --tyba-text-muted

# Onde o bundler larga os dois ícones. O chevron mora no meio deles.
APP_X, ICONE_Y = _DMG["appPosition"]["x"], _DMG["appPosition"]["y"]
PASTA_X = _DMG["applicationFolderPosition"]["x"]

# Via `Path.home()`: o caminho absoluto do autor não tem por que viajar num
# repositório público — mesma resolução, sem o username no diff.
FONTE = str(Path.home() / "Library/Fonts/JetBrainsMono-{}.ttf")


def desenha() -> Image.Image:
    img = Image.new("RGB", (LARGURA, ALTURA), SURFACE)
    d = ImageDraw.Draw(img)

    # `--tyba-surface` caindo para `--tyba-sunken`: a mesma queda que separa o
    # painel da área de terminal no app. Dez níveis em quatrocentos pixels —
    # perceptível como superfície, longe de virar cinza.
    #
    # O gradiente fecha em VISIVEL, não em ALTURA: seu ponto final tem de cair
    # DENTRO da janela, senão a queda que se desenhou não é a que se vê.
    for y in range(VISIVEL):
        t = (y / (VISIVEL - 1)) ** 1.6
        d.line(
            [(0, y), (LARGURA, y)],
            fill=tuple(round(SURFACE[c] + (SUNKEN[c] - SURFACE[c]) * t) for c in range(3)),
        )
    # As linhas abaixo do corte nunca aparecem. Continuam na cor final em vez de
    # ficarem no `SURFACE` do `Image.new` — assim, se um macOS futuro mudar a
    # altura da barra e revelar essa faixa, ela é continuação, não emenda.
    for y in range(VISIVEL, ALTURA):
        d.line([(0, y), (LARGURA, y)], fill=SUNKEN)

    # O CLARO ATRÁS DO RÓTULO — e isto não é escolha estética, é legibilidade.
    #
    # A janela de DMG com imagem de fundo desenha os rótulos dos ícones em PRETO
    # mesmo com o sistema em modo escuro: conferido por captura da janela real,
    # luminância 1,0 de texto sobre 5,0 de fundo, num Mac com
    # `AppleInterfaceStyle = Dark`. Fundo escuro apaga "Tyba" e "Applications"
    # SEMPRE, e não é variação de máquina que dê para aceitar.
    #
    # Duas tentativas anteriores e por que caíram:
    #
    # - Espalhar luz sob os ícones virou névoa: comeu a metade de baixo dos dois.
    # - Dividir a janela em dois temas — escuro em cima, claro do corte até a
    #   base — deu contraste, mas a área clara ficou com 27% da janela e o dono
    #   do produto leu a própria tela como "o fundo não pegou tudo". Se quem
    #   desenhou lê como bug, o usuário lê também.
    #
    # A luz é só o que o rótulo pede: uma pastilha atrás de cada um, e o resto da
    # janela continua preto. Sem faixa, não há região onde o fundo "acaba".
    for centro_x, largura in (
        (APP_X, CHIP_LARGURA_APP),
        (PASTA_X, CHIP_LARGURA_PASTA),
    ):
        caixa = [
            centro_x - largura / 2,
            ROTULO_Y - CHIP_ALTURA / 2,
            centro_x + largura / 2,
            ROTULO_Y + CHIP_ALTURA / 2,
        ]
        d.rounded_rectangle(caixa, radius=CHIP_ALTURA / 2, fill=CLARO_BG)
        d.rounded_rectangle(caixa, radius=CHIP_ALTURA / 2, outline=CHIP_BORDA, width=1)

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
    #
    # Vive no ESCURO, e é o que dispensou a faixa clara. Este texto é desenhado
    # aqui, então a cor é nossa: `--tyba-text-muted` sobre preto. Só os rótulos do
    # Finder são pretos por imposição dele. Enquanto a legenda precisasse de claro
    # junto com eles, o claro tinha de virar região, não pastilha.
    #
    # Ancorada entre a base da pastilha e o fim da faixa VISÍVEL, não da altura da
    # imagem: mudar a altura da janela na config não pode empurrá-la para baixo do
    # corte da barra de título, onde nada é visto.
    d.text(
        (
            LARGURA / 2,
            (ROTULO_Y + CHIP_ALTURA / 2)
            + (VISIVEL - (ROTULO_Y + CHIP_ALTURA / 2)) * 0.55,
        ),
        "Arraste o Tyba para Aplicativos",
        font=ImageFont.truetype(FONTE.format("Regular"), 11),
        fill=MUTED,
        anchor="mm",
    )
    return img


if __name__ == "__main__":
    alvo = RAIZ / "src-tauri" / _DMG["background"]
    desenha().save(alvo)
    print(f"{alvo.relative_to(RAIZ)} · {LARGURA}x{ALTURA}")
