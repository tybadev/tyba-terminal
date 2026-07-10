use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Terminal,
    Gui,
}

struct Known {
    id: &'static str,
    name: &'static str,
    surface: Surface,
    wait_args: &'static [&'static str],
    bins: &'static [&'static str],
    bundles: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Editor {
    pub id: String,
    pub name: String,
    pub path: String,
    pub terminal: bool,
}

const TOOLBOX_SCRIPTS: &str = "Library/Application Support/JetBrains/Toolbox/scripts";

const KNOWN: &[Known] = &[
    Known {
        id: "vscode",
        name: "Visual Studio Code",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["code"],
        bundles: &["/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"],
    },
    Known {
        id: "cursor",
        name: "Cursor",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["cursor"],
        bundles: &["/Applications/Cursor.app/Contents/Resources/app/bin/cursor"],
    },
    Known {
        id: "windsurf",
        name: "Windsurf",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["windsurf"],
        bundles: &["/Applications/Windsurf.app/Contents/Resources/app/bin/windsurf"],
    },
    Known {
        id: "zed",
        name: "Zed",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["zed"],
        bundles: &["/Applications/Zed.app/Contents/MacOS/cli"],
    },
    Known {
        id: "sublime",
        name: "Sublime Text",
        surface: Surface::Gui,
        wait_args: &["-w"],
        bins: &["subl"],
        bundles: &["/Applications/Sublime Text.app/Contents/SharedSupport/bin/subl"],
    },
    Known {
        id: "idea",
        name: "IntelliJ IDEA",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["idea"],
        bundles: &[],
    },
    Known {
        id: "pycharm",
        name: "PyCharm",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["pycharm"],
        bundles: &[],
    },
    Known {
        id: "webstorm",
        name: "WebStorm",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["webstorm"],
        bundles: &[],
    },
    Known {
        id: "goland",
        name: "GoLand",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["goland"],
        bundles: &[],
    },
    Known {
        id: "rustrover",
        name: "RustRover",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["rustrover"],
        bundles: &[],
    },
    Known {
        id: "datagrip",
        name: "DataGrip",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["datagrip"],
        bundles: &[],
    },
    Known {
        id: "studio",
        name: "Android Studio",
        surface: Surface::Gui,
        wait_args: &["--wait"],
        bins: &["studio"],
        bundles: &[],
    },
    Known {
        id: "nvim",
        name: "Neovim",
        surface: Surface::Terminal,
        wait_args: &[],
        bins: &["nvim"],
        bundles: &[],
    },
    Known {
        id: "vim",
        name: "Vim",
        surface: Surface::Terminal,
        wait_args: &[],
        bins: &["vim"],
        bundles: &[],
    },
    Known {
        id: "helix",
        name: "Helix",
        surface: Surface::Terminal,
        wait_args: &[],
        bins: &["hx"],
        bundles: &[],
    },
    Known {
        id: "nano",
        name: "Nano",
        surface: Surface::Terminal,
        wait_args: &[],
        bins: &["nano"],
        bundles: &[],
    },
    Known {
        id: "emacs",
        name: "Emacs",
        surface: Surface::Terminal,
        wait_args: &["-nw"],
        bins: &["emacs"],
        bundles: &[],
    },
];

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn in_path(bin: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(bin))
        .find(|candidate| is_executable(candidate))
}

fn in_toolbox(bin: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let candidate = PathBuf::from(home).join(TOOLBOX_SCRIPTS).join(bin);
    is_executable(&candidate).then_some(candidate)
}

fn locate(known: &Known) -> Option<PathBuf> {
    for bin in known.bins {
        if let Some(found) = in_path(bin).or_else(|| in_toolbox(bin)) {
            return Some(found);
        }
    }
    known
        .bundles
        .iter()
        .map(PathBuf::from)
        .find(|candidate| is_executable(candidate))
}

pub fn detect() -> Vec<Editor> {
    KNOWN
        .iter()
        .filter_map(|known| {
            locate(known).map(|path| Editor {
                id: known.id.to_string(),
                name: known.name.to_string(),
                path: path.to_string_lossy().into_owned(),
                terminal: known.surface == Surface::Terminal,
            })
        })
        .collect()
}

fn quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', r"'\''"))
}

pub fn env_command(id: &str) -> Option<String> {
    let known = KNOWN.iter().find(|k| k.id == id)?;
    let path = locate(known)?;
    let mut command = quote(&path.to_string_lossy());
    for arg in known.wait_args {
        command.push(' ');
        command.push_str(arg);
    }
    Some(command)
}

#[cfg(test)]
mod known_table_tests {
    use super::{Surface, KNOWN};

    #[test]
    fn every_gui_editor_declares_a_wait_flag() {
        for known in KNOWN {
            if known.surface == Surface::Gui {
                assert!(
                    !known.wait_args.is_empty(),
                    "{} é GUI e não bloqueia sem flag de wait",
                    known.id
                );
            }
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<&str> = KNOWN.iter().map(|k| k.id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn every_entry_has_at_least_one_candidate() {
        for known in KNOWN {
            assert!(
                !known.bins.is_empty() || !known.bundles.is_empty(),
                "{} não tem como ser encontrado",
                known.id
            );
        }
    }
}

#[cfg(test)]
mod env_command_tests {
    use super::{env_command, quote};

    #[test]
    fn an_unknown_id_yields_no_command() {
        assert!(env_command("editor-que-nao-existe").is_none());
    }

    #[test]
    fn a_path_with_spaces_is_quoted() {
        assert_eq!(
            quote("/Applications/Visual Studio Code.app/bin/code"),
            "'/Applications/Visual Studio Code.app/bin/code'"
        );
    }

    #[test]
    fn a_single_quote_in_the_path_cannot_escape_the_quoting() {
        let quoted = quote("/tmp/ed'; rm -rf /; echo '");
        assert_eq!(quoted, r"'/tmp/ed'\''; rm -rf /; echo '\'''");
        assert!(!quoted.contains("; rm -rf /; echo ';"));
    }

    #[test]
    fn a_terminal_editor_gets_no_wait_flag() {
        if let Some(command) = env_command("nano") {
            assert!(!command.contains("--wait"));
            assert!(command.ends_with("nano'"));
        }
    }

    #[test]
    fn a_gui_editor_carries_its_wait_flag() {
        if let Some(command) = env_command("vscode") {
            assert!(command.ends_with(" --wait"));
        }
    }
}

#[cfg(all(test, unix))]
mod detect_tests {
    use super::{detect, is_executable};

    #[test]
    fn a_non_executable_file_is_never_a_candidate() {
        let file = std::env::temp_dir().join(format!("tyba-noexec-{}", uuid::Uuid::new_v4()));
        std::fs::write(&file, "x").unwrap();
        assert!(!is_executable(&file));
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn a_directory_is_never_a_candidate() {
        assert!(!is_executable(&std::env::temp_dir()));
    }

    #[test]
    fn every_detected_editor_points_at_an_executable() {
        for editor in detect() {
            assert!(
                is_executable(std::path::Path::new(&editor.path)),
                "{} aponta para {} que não é executável",
                editor.id,
                editor.path
            );
        }
    }
}
