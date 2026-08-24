//! Onde estão os arquivos de histórico e quantas entradas cada um tem.
//!
//! Separado do runner porque contar é o que alimenta o convite de primeiro uso:
//! mostrar "achei 47 mil comandos" **sem** gravar nada antes do aceite.

use std::fs;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::parser;
use super::ParsedEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportSource {
    Zsh,
    Bash,
    Fish,
}

impl ImportSource {
    /// Prefixo da chave de import. Fixo: mudar isto faz o histórico já
    /// importado deixar de casar consigo mesmo e duplicar no próximo import.
    pub fn key(self) -> &'static str {
        match self {
            ImportSource::Zsh => "zsh",
            ImportSource::Bash => "bash",
            ImportSource::Fish => "fish",
        }
    }
}

/// Uma fonte que existe no disco, com a data do arquivo já lida — é dela que
/// sai a data sintetizada das entradas sem timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSource {
    pub source: ImportSource,
    pub path: PathBuf,
    pub mtime_ms: i64,
}

/// O que o convite e a tela de Configurações mostram antes de qualquer escrita.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceScan {
    pub source: ImportSource,
    pub path: String,
    pub entries: usize,
}

/// As fontes que existem, sem duplicata de caminho.
///
/// `$HISTFILE` é uma variável só, e vale para o shell que a definiu: aplicá-la a
/// zsh **e** a bash importaria o mesmo arquivo duas vezes, com a contagem de uso
/// dobrada. Por isso ela só vale para o shell que `$SHELL` aponta; os outros vão
/// pelo caminho padrão. Na prática ela quase nunca chega até aqui, porque é
/// definida no rc do shell e não exportada.
pub fn resolve(home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<ResolvedSource> {
    let histfile = env("HISTFILE").filter(|value| !value.is_empty());
    let shell = env("SHELL").unwrap_or_default();
    let for_shell = |name: &str| -> Option<PathBuf> {
        if shell.rsplit('/').next() == Some(name) {
            histfile.as_deref().map(PathBuf::from)
        } else {
            None
        }
    };

    let fish_default = env("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("fish/fish_history");

    let candidates = [
        (
            ImportSource::Zsh,
            for_shell("zsh").unwrap_or_else(|| home.join(".zsh_history")),
        ),
        (
            ImportSource::Bash,
            for_shell("bash").unwrap_or_else(|| home.join(".bash_history")),
        ),
        (ImportSource::Fish, fish_default),
    ];

    let mut resolved: Vec<ResolvedSource> = Vec::new();
    for (source, path) in candidates {
        if resolved.iter().any(|other| other.path == path) {
            continue;
        }
        if let Some(mtime_ms) = mtime_ms(&path) {
            resolved.push(ResolvedSource {
                source,
                path,
                mtime_ms,
            });
        }
    }
    resolved
}

/// Conta as entradas da fonte. Não escreve nada em lugar nenhum.
pub fn scan(source: &ResolvedSource) -> io::Result<SourceScan> {
    let reader = BufReader::new(fs::File::open(&source.path)?);
    let entries = match source.source {
        ImportSource::Zsh => parser::zsh::count(reader)?,
        ImportSource::Bash => parser::bash::count(reader)?,
        ImportSource::Fish => parser::fish::count(reader)?,
    };
    Ok(SourceScan {
        source: source.source,
        path: source.path.to_string_lossy().into_owned(),
        entries,
    })
}

/// Lê a fonte inteira, entregando entrada por entrada. Devolve quantas foram
/// descartadas por não decodificarem.
pub fn read(
    source: &ResolvedSource,
    total: usize,
    on_entry: &mut dyn FnMut(ParsedEntry),
) -> io::Result<usize> {
    let reader = BufReader::new(fs::File::open(&source.path)?);
    match source.source {
        ImportSource::Zsh => parser::zsh::parse(reader, source.mtime_ms, total, on_entry),
        ImportSource::Bash => parser::bash::parse(reader, source.mtime_ms, total, on_entry),
        ImportSource::Fish => parser::fish::parse(reader, source.mtime_ms, total, on_entry),
    }
}

fn mtime_ms(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let since_epoch = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_millis()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name: &str| {
            owned
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn only_sources_that_exist_are_resolved() {
        let home = tempfile::tempdir().unwrap();
        write(&home.path().join(".zsh_history"), "ls\n");

        let found = resolve(home.path(), &env_from(&[]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, ImportSource::Zsh);
        assert_eq!(found[0].path, home.path().join(".zsh_history"));
    }

    #[test]
    fn fish_follows_xdg_data_home_when_it_is_set() {
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        write(&data.path().join("fish/fish_history"), "- cmd: ls\n");

        let env = env_from(&[("XDG_DATA_HOME", data.path().to_str().unwrap())]);
        let found = resolve(home.path(), &env);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, ImportSource::Fish);
    }

    /// `$HISTFILE` pertence ao shell que a definiu.
    #[test]
    fn histfile_applies_only_to_the_shell_that_shell_points_at() {
        let home = tempfile::tempdir().unwrap();
        let custom = home.path().join("guardado/hist");
        write(&custom, "ls\n");
        write(&home.path().join(".bash_history"), "pwd\n");

        let env = env_from(&[
            ("HISTFILE", custom.to_str().unwrap()),
            ("SHELL", "/bin/zsh"),
        ]);
        let found = resolve(home.path(), &env);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].source, ImportSource::Zsh);
        assert_eq!(found[0].path, custom);
        assert_eq!(found[1].source, ImportSource::Bash);
        assert_eq!(found[1].path, home.path().join(".bash_history"));
    }

    /// Sem isto, `HISTFILE=~/.bash_history` com `SHELL=/bin/bash` entraria como
    /// bash e de novo pelo caminho padrão, dobrando a contagem de uso.
    #[test]
    fn the_same_path_is_never_resolved_twice() {
        let home = tempfile::tempdir().unwrap();
        let shared = home.path().join(".bash_history");
        write(&shared, "ls\n");

        let env = env_from(&[
            ("HISTFILE", shared.to_str().unwrap()),
            ("SHELL", "/bin/bash"),
        ]);
        let found = resolve(home.path(), &env);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, shared);
    }

    #[test]
    fn scan_counts_entries_without_writing_anything() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".zsh_history");
        write(&path, ": 1:0;ls\n: 2:0;pwd\n");
        let before = fs::metadata(&path).unwrap().len();

        let found = resolve(home.path(), &env_from(&[]));
        let scanned = scan(&found[0]).unwrap();
        assert_eq!(scanned.entries, 2);
        assert_eq!(scanned.source, ImportSource::Zsh);
        assert_eq!(fs::metadata(&path).unwrap().len(), before);
    }

    #[test]
    fn read_dispatches_to_the_parser_of_the_source() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".local/share/fish/fish_history"),
            "- cmd: echo oi\n  when: 7\n",
        );

        let found = resolve(home.path(), &env_from(&[]));
        let total = scan(&found[0]).unwrap().entries;
        let mut entries = Vec::new();
        let discarded = read(&found[0], total, &mut |entry| entries.push(entry)).unwrap();
        assert_eq!(discarded, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "echo oi");
        assert_eq!(entries[0].started_at_ms, 7_000);
    }

    #[test]
    fn a_missing_file_is_not_a_source() {
        let home = tempfile::tempdir().unwrap();
        assert!(resolve(home.path(), &env_from(&[])).is_empty());
    }
}
