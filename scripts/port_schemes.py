#!/usr/bin/env python3
"""Gera src-tauri/src/theme/schemes.rs a partir dos esquemas clássicos do design system.

Fonte: tyba-design-system/ds-bundle/tokens/themes.css (mesma fonte do port_styles.py).
Convenção ANSI da casa (igual aos built-ins TYBA):
  normal  = acento do tema (red/green/amber->yellow/blue/magenta/cyan)
  bright  = acento clareado 18% em direção ao branco
  brightGreen = lime · brightMagenta = violet
  dark  base: black = surface, white = text
  light base: black = text,    white = sunken
  brightBlack = textFaint · brightWhite = #ffffff
  cursor = green · selection = green com alpha 4d
"""
import pathlib
import re
import shutil
import subprocess
import sys

DS = pathlib.Path('/Users/guilherme/swell-system/tyba-design-system/ds-bundle/tokens/themes.css')
OUT = pathlib.Path('/Users/guilherme/swell-system/tyba-terminal/src-tauri/src/theme/schemes.rs')

NAMES = {
    'solarized-dark': 'Solarized Dark',
    'solarized-light': 'Solarized Light',
    'dracula': 'Dracula',
    'dracula-light': 'Dracula Light (Alucard)',
    'gruvbox-dark': 'Gruvbox Dark',
    'gruvbox-light': 'Gruvbox Light',
    'github-dark': 'GitHub Dark',
    'github-light': 'GitHub Light',
    'monokai': 'Monokai',
    'monokai-pro': 'Monokai Pro',
    'monokai-machine': 'Monokai Pro Machine',
    'monokai-octagon': 'Monokai Pro Octagon',
    'monokai-ristretto': 'Monokai Pro Ristretto',
    'monokai-spectrum': 'Monokai Pro Spectrum',
    'monokai-light': 'Monokai Pro Light',
}

HEX = re.compile(r'#[0-9a-fA-F]{6}\b')
BLOCK = re.compile(r"\[data-theme='([^']+)'\]\s*\{(.*?)\n\}", re.S)
TOKEN = re.compile(r'--tyba-([a-z-]+):\s*(#[0-9a-fA-F]{6})\s*;?')
SCHEME = re.compile(r'color-scheme:\s*(dark|light)')


def lighten(hex_color: str, amount: float = 0.18) -> str:
    r, g, b = (int(hex_color[i:i + 2], 16) for i in (1, 3, 5))
    mix = lambda c: round(c + (255 - c) * amount)
    return f'#{mix(r):02x}{mix(g):02x}{mix(b):02x}'


def palette(tokens: dict, base: str) -> dict:
    t = tokens
    if base == 'dark':
        black, white = t['surface'], t['text']
    else:
        black, white = t['text'], t['sunken']
    return {
        'background': t['bg'],
        'foreground': t['text'],
        'cursor': t['green'],
        'cursor_accent': t['bg'],
        'selection_background': t['green'] + '4d',
        'ansi': [
            black, t['red'], t['green'], t['amber'],
            t['blue'], t['magenta'], t['cyan'], white,
            t['text-faint'], lighten(t['red']), t['lime'], lighten(t['amber']),
            lighten(t['blue']), t['violet'], lighten(t['cyan']), '#ffffff',
        ],
    }


def rust_scheme(scheme_id: str, base: str, pal: dict) -> str:
    ansi_rows = []
    for i in range(0, 16, 4):
        row = ', '.join(f'"{c}"' for c in pal['ansi'][i:i + 4])
        ansi_rows.append(f'                {row},')
    ansi_block = '\n'.join(ansi_rows)
    base_variant = 'Dark' if base == 'dark' else 'Light'
    return f'''        scheme(
            "{scheme_id}",
            "{NAMES[scheme_id]}",
            ThemeBase::{base_variant},
            TerminalPalette {{
                background: "{pal['background']}".into(),
                foreground: "{pal['foreground']}".into(),
                cursor: "{pal['cursor']}".into(),
                cursor_accent: Some("{pal['cursor_accent']}".into()),
                selection_background: Some("{pal['selection_background']}".into()),
                ansi: ansi([
{ansi_block}
                ]),
            }},
        ),'''


def main() -> None:
    css = DS.read_text()
    blocks = BLOCK.findall(css)
    if len(blocks) != len(NAMES):
        sys.exit(f'esperava {len(NAMES)} blocos em themes.css, achei {len(blocks)} — revisar port_schemes.py')

    ids = []
    entries = []
    for scheme_id, body in blocks:
        if scheme_id not in NAMES:
            sys.exit(f'esquema desconhecido em themes.css: {scheme_id} — atualizar NAMES')
        base_match = SCHEME.search(body)
        if not base_match:
            sys.exit(f'{scheme_id}: sem color-scheme no bloco')
        base = base_match.group(1)
        tokens = dict(TOKEN.findall(body))
        missing = {'bg', 'surface', 'sunken', 'text', 'text-faint', 'green', 'lime',
                   'amber', 'magenta', 'violet', 'blue', 'cyan', 'red'} - tokens.keys()
        if missing:
            sys.exit(f'{scheme_id}: tokens ausentes {sorted(missing)}')
        ids.append(scheme_id)
        entries.append(rust_scheme(scheme_id, base, palette(tokens, base)))

    id_list = '\n'.join(f'    "{i}",' for i in ids)
    body = '\n'.join(entries)
    OUT.write_text(f'''//! Gerado por scripts/port_schemes.py a partir de tyba-design-system/ds-bundle/tokens/themes.css — nao editar na mao.

use std::collections::BTreeMap;

use super::{{ansi, TerminalPalette, Theme, ThemeBase}};

pub const IDS: &[&str] = &[
{id_list}
];

pub fn all() -> Vec<Theme> {{
    vec![
{body}
    ]
}}

fn scheme(id: &str, name: &str, base: ThemeBase, terminal: TerminalPalette) -> Theme {{
    Theme {{
        id: id.into(),
        name: name.into(),
        base,
        builtin: true,
        ui: BTreeMap::new(),
        terminal,
    }}
}}
''')
    rustfmt = shutil.which('rustfmt')
    if rustfmt:
        subprocess.run([rustfmt, '--edition', '2021', str(OUT)], check=True)
    else:
        print('aviso: rustfmt não encontrado — rode cargo fmt antes de commitar', file=sys.stderr)
    print(f'{OUT} gerado com {len(ids)} esquemas')


if __name__ == '__main__':
    main()
