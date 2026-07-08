//! Gerado por scripts/port_schemes.py a partir de tyba-design-system/ds-bundle/tokens/themes.css — nao editar na mao.

use std::collections::BTreeMap;

use super::{ansi, TerminalPalette, Theme, ThemeBase};

pub const IDS: &[&str] = &[
    "solarized-dark",
    "solarized-light",
    "dracula",
    "dracula-light",
    "gruvbox-dark",
    "gruvbox-light",
    "github-dark",
    "github-light",
    "monokai",
    "monokai-pro",
    "monokai-machine",
    "monokai-octagon",
    "monokai-ristretto",
    "monokai-spectrum",
    "monokai-light",
];

pub fn all() -> Vec<Theme> {
    vec![
        scheme(
            "solarized-dark",
            "Solarized Dark",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#002b36".into(),
                foreground: "#93a1a1".into(),
                cursor: "#859900".into(),
                cursor_accent: Some("#002b36".into()),
                selection_background: Some("#8599004d".into()),
                ansi: ansi([
                    "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198",
                    "#93a1a1", "#586e75", "#e25754", "#a3bf00", "#c29e2e", "#4da0da", "#6c71c4",
                    "#50b2ab", "#ffffff",
                ]),
            },
        ),
        scheme(
            "solarized-light",
            "Solarized Light",
            ThemeBase::Light,
            TerminalPalette {
                background: "#f5efdc".into(),
                foreground: "#073642".into(),
                cursor: "#859900".into(),
                cursor_accent: Some("#f5efdc".into()),
                selection_background: Some("#8599004d".into()),
                ansi: ansi([
                    "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198",
                    "#eee8d5", "#93a1a1", "#e25754", "#98ad10", "#c29e2e", "#4da0da", "#6c71c4",
                    "#50b2ab", "#ffffff",
                ]),
            },
        ),
        scheme(
            "dracula",
            "Dracula",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#282a36".into(),
                foreground: "#f8f8f2".into(),
                cursor: "#50fa7b".into(),
                cursor_accent: Some("#282a36".into()),
                selection_background: Some("#50fa7b4d".into()),
                ansi: ansi([
                    "#2f313f", "#ff5555", "#50fa7b", "#ffb86c", "#7b9eff", "#ff79c6", "#8be9fd",
                    "#f8f8f2", "#6272a4", "#ff7474", "#7dfc9d", "#ffc586", "#93afff", "#bd93f9",
                    "#a0edfd", "#ffffff",
                ]),
            },
        ),
        scheme(
            "dracula-light",
            "Dracula Light (Alucard)",
            ThemeBase::Light,
            TerminalPalette {
                background: "#f8f3de".into(),
                foreground: "#1f1f1f".into(),
                cursor: "#14710a".into(),
                cursor_accent: Some("#f8f3de".into()),
                selection_background: Some("#14710a4d".into()),
                ansi: ansi([
                    "#1f1f1f", "#cb3a2a", "#14710a", "#a34d14", "#2f5fc9", "#a3144d", "#036a96",
                    "#efead5", "#918c73", "#d45d50", "#3d8f2a", "#b46d3e", "#547cd3", "#644ac9",
                    "#3085a9", "#ffffff",
                ]),
            },
        ),
        scheme(
            "gruvbox-dark",
            "Gruvbox Dark",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#282828".into(),
                foreground: "#ebdbb2".into(),
                cursor: "#b8bb26".into(),
                cursor_accent: Some("#282828".into()),
                selection_background: Some("#b8bb264d".into()),
                ansi: ansi([
                    "#32302f", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c",
                    "#ebdbb2", "#928374", "#fc6a59", "#d5dc3a", "#fbc954", "#99b5ab", "#b16286",
                    "#a2cb94", "#ffffff",
                ]),
            },
        ),
        scheme(
            "gruvbox-light",
            "Gruvbox Light",
            ThemeBase::Light,
            TerminalPalette {
                background: "#f2e5bc".into(),
                foreground: "#3c3836".into(),
                cursor: "#79740e".into(),
                cursor_accent: Some("#f2e5bc".into()),
                selection_background: Some("#79740e4d".into()),
                ansi: ansi([
                    "#3c3836", "#9d0006", "#79740e", "#b57614", "#076678", "#b16286", "#427b58",
                    "#ebdbb2", "#928374", "#af2e33", "#98971a", "#c28f3e", "#348290", "#8f3f71",
                    "#649376", "#ffffff",
                ]),
            },
        ),
        scheme(
            "github-dark",
            "GitHub Dark",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#0d1117".into(),
                foreground: "#e6edf3".into(),
                cursor: "#3fb950".into(),
                cursor_accent: Some("#0d1117".into()),
                selection_background: Some("#3fb9504d".into()),
                ansi: ansi([
                    "#161b22", "#f85149", "#3fb950", "#d29922", "#58a6ff", "#db61a2", "#39c5cf",
                    "#e6edf3", "#6e7681", "#f9706a", "#56d364", "#daab4a", "#76b6ff", "#a371f7",
                    "#5dcfd8", "#ffffff",
                ]),
            },
        ),
        scheme(
            "github-light",
            "GitHub Light",
            ThemeBase::Light,
            TerminalPalette {
                background: "#f6f8fa".into(),
                foreground: "#1f2328".into(),
                cursor: "#1a7f37".into(),
                cursor_accent: Some("#f6f8fa".into()),
                selection_background: Some("#1a7f374d".into()),
                ansi: ansi([
                    "#1f2328", "#cf222e", "#1a7f37", "#9a6700", "#0969da", "#bf3989", "#1b7c83",
                    "#eaeef2", "#8c959f", "#d84a54", "#2da44e", "#ac822e", "#3584e1", "#8250df",
                    "#449499", "#ffffff",
                ]),
            },
        ),
        scheme(
            "monokai",
            "Monokai",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#272822".into(),
                foreground: "#f8f8f2".into(),
                cursor: "#a6e22e".into(),
                cursor_accent: Some("#272822".into()),
                selection_background: Some("#a6e22e4d".into()),
                ansi: ansi([
                    "#2e2f28", "#fc4c3f", "#a6e22e", "#fd971f", "#66d9ef", "#f92672", "#a1efe4",
                    "#f8f8f2", "#75715e", "#fd6c62", "#c4f04a", "#fdaa47", "#82e0f2", "#ae81ff",
                    "#b2f2e9", "#ffffff",
                ]),
            },
        ),
        scheme(
            "monokai-pro",
            "Monokai Pro",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#2d2a2e".into(),
                foreground: "#fcfcfa".into(),
                cursor: "#a9dc76".into(),
                cursor_accent: Some("#2d2a2e".into()),
                selection_background: Some("#a9dc764d".into()),
                ansi: ansi([
                    "#353236", "#f4485c", "#a9dc76", "#fc9867", "#78dce8", "#ff6188", "#7ce8cd",
                    "#fcfcfa", "#939293", "#f66979", "#c0e392", "#fdab82", "#90e2ec", "#ab9df2",
                    "#94ecd6", "#ffffff",
                ]),
            },
        ),
        scheme(
            "monokai-machine",
            "Monokai Pro Machine",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#273136".into(),
                foreground: "#f2fffc".into(),
                cursor: "#a2e57b".into(),
                cursor_accent: Some("#273136".into()),
                selection_background: Some("#a2e57b4d".into()),
                ansi: ansi([
                    "#2d3a40", "#f4536a", "#a2e57b", "#ffb270", "#7cd5f1", "#ff6d7e", "#7fe0ce",
                    "#f2fffc", "#6b7678", "#f67285", "#baf090", "#ffc08a", "#94ddf4", "#baa0f8",
                    "#96e6d7", "#ffffff",
                ]),
            },
        ),
        scheme(
            "monokai-octagon",
            "Monokai Pro Octagon",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#282a3a".into(),
                foreground: "#eaf2f1".into(),
                cursor: "#bad761".into(),
                cursor_accent: Some("#282a3a".into()),
                selection_background: Some("#bad7614d".into()),
                ansi: ansi([
                    "#2f3142", "#f4506b", "#bad761", "#ff9b5e", "#9cd1bb", "#ff657a", "#8ce0d8",
                    "#eaf2f1", "#696d77", "#f67086", "#d2e87e", "#ffad7b", "#aed9c7", "#c39ac9",
                    "#a1e6df", "#ffffff",
                ]),
            },
        ),
        scheme(
            "monokai-ristretto",
            "Monokai Pro Ristretto",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#2c2525".into(),
                foreground: "#fff1f3".into(),
                cursor: "#adda78".into(),
                cursor_accent: Some("#2c2525".into()),
                selection_background: Some("#adda784d".into()),
                ansi: ansi([
                    "#342c2c", "#f04a60", "#adda78", "#f38d70", "#85dacc", "#fd6883", "#7fe4c8",
                    "#fff1f3", "#72696a", "#f36b7d", "#c5ea90", "#f5a28a", "#9be1d5", "#a8a9eb",
                    "#96e9d2", "#ffffff",
                ]),
            },
        ),
        scheme(
            "monokai-spectrum",
            "Monokai Pro Spectrum",
            ThemeBase::Dark,
            TerminalPalette {
                background: "#222222".into(),
                foreground: "#f7f1ff".into(),
                cursor: "#7bd88f".into(),
                cursor_accent: Some("#222222".into()),
                selection_background: Some("#7bd88f4d".into()),
                ansi: ansi([
                    "#2a2a2b", "#f0485e", "#7bd88f", "#fd9353", "#5ad4e6", "#fc618d", "#62e2cc",
                    "#f7f1ff", "#69676c", "#f3697b", "#97e5a6", "#fda672", "#78dcea", "#948ae3",
                    "#7ee7d5", "#ffffff",
                ]),
            },
        ),
        scheme(
            "monokai-light",
            "Monokai Pro Light",
            ThemeBase::Light,
            TerminalPalette {
                background: "#faf4f2".into(),
                foreground: "#29242a".into(),
                cursor: "#269d69".into(),
                cursor_accent: Some("#faf4f2".into()),
                selection_background: Some("#269d694d".into()),
                ansi: ansi([
                    "#29242a", "#d0342c", "#269d69", "#cc7a0a", "#1c8ca8", "#e14775", "#1da08a",
                    "#ede7e5", "#a59fa0", "#d85952", "#4db07e", "#d59236", "#45a1b8", "#7058be",
                    "#46b19f", "#ffffff",
                ]),
            },
        ),
    ]
}

fn scheme(id: &str, name: &str, base: ThemeBase, terminal: TerminalPalette) -> Theme {
    Theme {
        id: id.into(),
        name: name.into(),
        base,
        builtin: true,
        ui: BTreeMap::new(),
        terminal,
    }
}
