//! Os comandos que existem na máquina — a fonte do PRIMEIRO token.
//!
//! O resto da completação sabe continuar uma linha: `complete_path` completa o
//! caminho, `argument` completa subcomando e flag, o histórico completa a linha
//! inteira. Nenhum deles responde à primeira palavra vinda de fora do histórico,
//! e `next_tokens` recusa prefixo vazio de propósito. Numa máquina onde `pnpm`
//! nunca foi digitado, `pn` devolvia silêncio.
//!
//! Aqui a fonte é o disco: os diretórios do `$PATH`, lidos pelo core. Alias e
//! função NÃO passam por aqui — não existem em disco, e quem os conhece é o
//! shell (ver a tech spec 02 no cofre).

use std::path::Path;

/// O arquivo pode ser executado por alguém?
///
/// Basta um dos três bits: um binário instalado por outro usuário costuma vir
/// `0o755`, e exigir o bit do dono esconderia metade do `/usr/bin`. Quem não
/// puder executar descobre no `exec`, não na sugestão — errar para o lado de
/// mostrar é melhor do que sumir com um comando que existe.
#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}

/// No Windows não há bit de execução: quem decide é a extensão.
#[cfg(not(unix))]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    meta.is_file()
}

/// Nomes de executável encontrados nos diretórios, sem repetir.
pub fn scan(dirs: &[&Path]) -> Vec<String> {
    let mut found = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // `metadata()` segue link simbólico de propósito: metade do
            // `/usr/local/bin` de um Mac com Homebrew é link para a Cellar, e
            // olhar o link em vez do alvo esconderia tudo isso.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !is_executable(&meta) {
                continue;
            }
            found.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Cria um arquivo com o modo pedido — `0o755` executável, `0o644` não.
    fn file(dir: &Path, name: &str, mode: u32) {
        let path = dir.join(name);
        std::fs::write(&path, "").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(mode);
        std::fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn finds_the_executable_and_ignores_what_cannot_run() {
        // O `$PATH` tem README, LICENSE e afins em diretório de pacote. Listá-los
        // como comando é oferecer algo que não roda.
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "tyba", 0o755);
        file(dir.path(), "README.md", 0o644);

        assert_eq!(scan(&[dir.path()]), vec!["tyba"]);
    }
}
