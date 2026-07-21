use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{open_verified, resolve_within};

pub fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn valid_component(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("nome vazio".into());
    }
    if name == "." || name == ".." {
        return Err("nome reservado".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("nome com separador de caminho rejeitado".into());
    }
    Ok(())
}

fn split_leaf(rel: &str) -> Result<(String, String), String> {
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() {
        return Err("caminho vazio".into());
    }
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err("path absoluto rejeitado".into());
    }
    for comp in path.components() {
        match comp {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            _ => return Err("path traversal rejeitado".into()),
        }
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("nome inválido")?
        .to_string();
    valid_component(&name)?;
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    Ok((parent, name))
}

/// Resolve o path de destino de uma escrita/criação cujo alvo pode ainda não
/// existir: o **pai** é validado contra a raiz (canonicalizado, sem symlink que
/// escape), e a folha é anexada crua. O alvo em si não é canonicalizado porque
/// pode não existir — a revalidação de fd cobre o alvo existente na escrita.
pub fn resolve_target_within(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let (parent_rel, name) = split_leaf(rel)?;
    let parent = resolve_within(root, &parent_rel)?;
    if !parent.is_dir() {
        return Err("diretório de destino inexistente".into());
    }
    Ok(parent.join(name))
}

fn read_current(root: &Path, target: &Path) -> Result<Option<Vec<u8>>, String> {
    if !target.exists() {
        return Ok(None);
    }
    let (mut file, _real) = open_verified(root, target)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| format!("falha ao ler conteúdo atual: {e}"))?;
    Ok(Some(buf))
}

fn atomic_write(root: &Path, target: &Path, content: &[u8]) -> Result<(), String> {
    let parent = target.parent().ok_or("destino sem diretório pai")?;
    if target.exists() {
        let (_file, _real) = open_verified(root, target)?;
    }
    let tmp = parent.join(format!(".tyba-write-{}", uuid::Uuid::new_v4()));
    let write_result = (|| -> Result<(), String> {
        let mut file =
            std::fs::File::create(&tmp).map_err(|e| format!("falha ao criar temporário: {e}"))?;
        file.write_all(content)
            .map_err(|e| format!("falha ao escrever: {e}"))?;
        file.sync_all().map_err(|e| format!("falha no sync: {e}"))?;
        Ok(())
    })();
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("falha ao promover temporário: {e}"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WriteResult {
    Written { hash: String },
    Conflict { disk_hash: Option<String> },
}

/// Guarda de conflito no core: só escreve se o hash do disco ainda bate com o
/// esperado (capturado no load). Divergiu → recusa sem tocar disco e devolve o
/// hash atual pra que o "sobrescrever" (decisão humana) seja um novo write com
/// o hash certo.
pub fn write_file(
    root: &Path,
    rel: &str,
    content: &str,
    expected_hash: &str,
) -> Result<WriteResult, String> {
    let target = resolve_target_within(root, rel)?;
    let current = read_current(root, &target)?;
    let disk_hash = current.as_deref().map(hash_bytes);
    let matches = match &disk_hash {
        Some(h) => h == expected_hash,
        None => expected_hash.is_empty(),
    };
    if !matches {
        return Ok(WriteResult::Conflict { disk_hash });
    }
    atomic_write(root, &target, content.as_bytes())?;
    Ok(WriteResult::Written {
        hash: hash_bytes(content.as_bytes()),
    })
}

pub fn create(root: &Path, rel: &str, is_dir: bool) -> Result<(), String> {
    let target = resolve_target_within(root, rel)?;
    if target.exists() {
        return Err("já existe um item com esse nome".into());
    }
    if is_dir {
        std::fs::create_dir(&target).map_err(|e| format!("falha ao criar pasta: {e}"))
    } else {
        atomic_write(root, &target, b"")
    }
}

pub fn rename(root: &Path, from_rel: &str, to_rel: &str) -> Result<(), String> {
    let from = resolve_within(root, from_rel)?;
    let to = resolve_target_within(root, to_rel)?;
    if from == to {
        return Ok(());
    }
    if to.exists() {
        return Err("já existe um item com esse nome".into());
    }
    std::fs::rename(&from, &to).map_err(|e| format!("falha ao renomear: {e}"))
}

pub fn delete(root: &Path, rel: &str) -> Result<(), String> {
    let target = resolve_within(root, rel)?;
    let (_file, real) = open_verified_any(root, &target)?;
    trash::delete(&real).map_err(|e| format!("falha ao mover para a Lixeira: {e}"))
}

/// Como `open_verified`, mas aceita diretório: valida o path real do fd (arquivo)
/// ou canonicaliza (diretório) contra a raiz. Fecha o mesmo TOCTOU de symlink da
/// leitura para o delete, que também opera sobre pastas.
fn open_verified_any(root: &Path, full: &Path) -> Result<(Option<std::fs::File>, PathBuf), String> {
    let root_canon = std::fs::canonicalize(root).map_err(|e| format!("raiz inacessível: {e}"))?;
    if full.is_dir() {
        let canon = std::fs::canonicalize(full).map_err(|e| format!("path inacessível: {e}"))?;
        if !canon.starts_with(&root_canon) {
            return Err("path fora da raiz do painel".into());
        }
        return Ok((None, canon));
    }
    let (file, real) = open_verified(root, full)?;
    Ok((Some(file), real))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-write-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    #[test]
    fn write_with_matching_hash_applies() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "old").unwrap();
        let expected = hash_bytes(b"old");
        let result = write_file(&root, "a.txt", "new content", &expected).unwrap();
        assert!(matches!(result, WriteResult::Written { .. }));
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "new content"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_with_divergent_hash_refuses_without_touching_disk() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "on disk now").unwrap();
        let stale = hash_bytes(b"what the ui loaded");
        let result = write_file(&root, "a.txt", "clobber", &stale).unwrap();
        match result {
            WriteResult::Conflict { disk_hash } => {
                assert_eq!(disk_hash, Some(hash_bytes(b"on disk now")));
            }
            other => panic!("esperava conflito, veio {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "on disk now",
            "o disco não pode ter sido tocado no conflito"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn overwrite_uses_the_current_disk_hash() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "agent wrote this").unwrap();
        let disk = hash_bytes(b"agent wrote this");
        let result = write_file(&root, "a.txt", "human wins", &disk).unwrap();
        assert!(matches!(result, WriteResult::Written { .. }));
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "human wins"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_is_atomic_no_partial_temp_left_behind() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        write_file(&root, "a.txt", "y".repeat(4096).as_str(), &hash_bytes(b"x")).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tyba-write-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporário não pode sobrar após o rename"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_through_escaping_symlink_is_refused() {
        let root = tmp();
        let outside = tmp();
        std::fs::write(outside.join("secret.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("escape")).unwrap();
        let err = write_file(&root, "escape", "leak", "").unwrap_err();
        assert!(
            err.contains("fora"),
            "esperava recusa por escape, veio {err}"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("secret.txt")).unwrap(),
            "outside",
            "o arquivo fora da raiz não pode ter sido escrito"
        );
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn create_rejects_traversal() {
        let root = tmp();
        assert!(create(&root, "../evil.txt", false).is_err());
        assert!(create(&root, "/etc/evil", false).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn create_makes_file_and_dir_with_utf8_and_dash_names() {
        let root = tmp();
        create(&root, "café .txt", false).unwrap();
        create(&root, "--flag-like", true).unwrap();
        assert!(root.join("café .txt").is_file());
        assert!(root.join("--flag-like").is_dir());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn create_rejects_existing_and_reserved_names() {
        let root = tmp();
        std::fs::write(root.join("dup"), "x").unwrap();
        assert!(create(&root, "dup", false).is_err());
        assert!(create(&root, "..", true).is_err());
        assert!(create(&root, ".", true).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn rename_into_escaping_symlink_dir_is_refused() {
        let root = tmp();
        let outside = tmp();
        std::os::unix::fs::symlink(&outside, root.join("out")).unwrap();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let err = rename(&root, "a.txt", "out/a.txt").unwrap_err();
        assert!(
            err.contains("fora"),
            "esperava recusa por escape, veio {err}"
        );
        assert!(root.join("a.txt").is_file(), "a origem não pode ter sumido");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn rename_moves_within_root_and_rejects_collision() {
        let root = tmp();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        rename(&root, "a.txt", "sub/b.txt").unwrap();
        assert!(root.join("sub/b.txt").is_file());
        assert!(!root.join("a.txt").exists());
        std::fs::write(root.join("c.txt"), "y").unwrap();
        assert!(rename(&root, "c.txt", "sub/b.txt").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn delete_moves_to_trash_and_rejects_escape() {
        let root = tmp();
        std::fs::write(root.join("gone.txt"), "bye").unwrap();
        if delete(&root, "gone.txt").is_ok() {
            assert!(!root.join("gone.txt").exists());
        }
        assert!(delete(&root, "../etc").is_err());
        std::fs::remove_dir_all(&root).ok();
    }
}
