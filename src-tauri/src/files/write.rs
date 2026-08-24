use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{open_verified, resolve_within, EDIT_MAX_BYTES};

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

/// Recusa alvos que resolveriam para a própria raiz do painel: `rel` vazio, `/`
/// ou `.`. Defensivo — nenhum command de escrita pode operar sobre a raiz.
fn reject_root_rel(rel: &str) -> Result<(), String> {
    let trimmed = rel.trim_start_matches('/').trim();
    if trimmed.is_empty() || trimmed == "." {
        return Err("operação sobre a raiz do painel rejeitada".into());
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

fn canon(root: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(root).map_err(|e| format!("raiz inacessível: {e}"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WriteResult {
    Written { hash: String },
    Conflict { disk_hash: Option<String> },
}

/// Guarda de conflito no core: só escreve se o hash do disco ainda bate com o
/// esperado (capturado no load). A checagem é um CAS — o hash é reconferido
/// imediatamente antes do rename (fecha a janela de clobber do agente); e a
/// mutação é relativa a um dirfd revalidado (fecha o TOCTOU de symlink no
/// diretório intermediário). Divergiu → recusa sem promover o temporário.
pub fn write_file(
    root: &Path,
    rel: &str,
    content: &str,
    expected_hash: &str,
) -> Result<WriteResult, String> {
    write_file_hooked(root, rel, content, expected_hash, |_| {})
}

fn write_file_hooked<F: FnOnce(&Path)>(
    root: &Path,
    rel: &str,
    content: &str,
    expected_hash: &str,
    before_rename: F,
) -> Result<WriteResult, String> {
    if content.len() as u64 > EDIT_MAX_BYTES {
        return Err("arquivo grande demais para salvar pelo painel".into());
    }
    reject_root_rel(rel)?;
    let (parent_rel, leaf) = split_leaf(rel)?;
    let parent = resolve_within(root, &parent_rel)?;
    let root_canon = canon(root)?;
    imp::write_atomic(
        &root_canon,
        &parent,
        &leaf,
        content,
        expected_hash,
        before_rename,
    )
}

pub fn create(root: &Path, rel: &str, is_dir: bool) -> Result<(), String> {
    reject_root_rel(rel)?;
    let (parent_rel, leaf) = split_leaf(rel)?;
    let parent = resolve_within(root, &parent_rel)?;
    let root_canon = canon(root)?;
    imp::create(&root_canon, &parent, &leaf, is_dir)
}

pub fn rename(root: &Path, from_rel: &str, to_rel: &str) -> Result<(), String> {
    reject_root_rel(from_rel)?;
    reject_root_rel(to_rel)?;
    let (from_parent_rel, from_leaf) = split_leaf(from_rel)?;
    let (to_parent_rel, to_leaf) = split_leaf(to_rel)?;
    let from_parent = resolve_within(root, &from_parent_rel)?;
    let to_parent = resolve_within(root, &to_parent_rel)?;
    if from_parent == to_parent && from_leaf == to_leaf {
        return Ok(());
    }
    let root_canon = canon(root)?;
    imp::rename(&root_canon, &from_parent, &from_leaf, &to_parent, &to_leaf)
}

/// Qual caminho o [`delete`] mandaria para a Lixeira — e o handle que mantém a
/// verificação de pé até a remoção acontecer.
///
/// Separado do `delete` porque é aqui que mora a garantia de segurança: **qual**
/// caminho é escolhido. Se a Lixeira funciona é outro assunto, e é um assunto
/// do sistema.
///
/// A separação é o que torna a garantia testável sem efeito colateral. O teste
/// que a cobria chamava a Lixeira de verdade e descartava o resultado com
/// `let _ =` — então ele provava pouco, e no macOS ainda abria um diálogo do
/// Finder pedindo Touch ID a cada `cargo test`. Teste que precisa de senha não
/// roda em CI e treina quem desenvolve a clicar em "permitir" sem ler.
fn delete_target(root: &Path, rel: &str) -> Result<(Option<std::fs::File>, PathBuf), String> {
    reject_root_rel(rel)?;
    let (parent_rel, leaf) = split_leaf(rel)?;
    let parent = resolve_within(root, &parent_rel)?;
    let root_canon = canon(root)?;
    let dir_real = imp::verified_dir_real(&root_canon, &parent)?;
    let link_path = dir_real.join(&leaf);
    let meta =
        std::fs::symlink_metadata(&link_path).map_err(|e| format!("item inexistente: {e}"))?;
    if meta.file_type().is_symlink() {
        // Symlink: vai o LINK pra Lixeira, nunca o alvo — vale mesmo que o alvo
        // aponte pra fora da raiz, porque o link vive na raiz, já validada.
        Ok((None, link_path))
    } else {
        open_verified_any(root, &link_path)
    }
}

pub fn delete(root: &Path, rel: &str) -> Result<(), String> {
    let (_handle, target) = delete_target(root, rel)?;
    trash::delete(&target).map_err(|e| format!("falha ao mover para a Lixeira: {e}"))
}

/// Como `open_verified`, mas aceita diretório: valida o path real do fd (arquivo)
/// ou canonicaliza (diretório) contra a raiz. Só usado no ramo não-symlink do
/// delete — o symlink é tratado sem seguir o alvo.
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

#[cfg(unix)]
mod imp {
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::path::{Path, PathBuf};

    use rustix::fs::{self, AtFlags, FileType, Mode, OFlags, RawMode};

    use super::hash_bytes;
    use super::WriteResult;

    /// Abre o diretório de destino e revalida o path REAL do dirfd contra a raiz
    /// canônica: se um componente intermediário virou symlink entre a validação
    /// e a abertura, o fd cai fora da raiz e a operação aborta.
    fn open_dir_verified(root_canon: &Path, parent: &Path) -> Result<OwnedFd, String> {
        let fd = fs::open(parent, OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())
            .map_err(|e| format!("falha ao abrir diretório de destino: {e}"))?;
        let real = super::super::fd_real_path_raw(fd.as_raw_fd())
            .ok_or("não foi possível revalidar o diretório de destino")?;
        let real = std::fs::canonicalize(&real).unwrap_or(real);
        if !real.starts_with(root_canon) {
            return Err("diretório de destino fora da raiz do painel".into());
        }
        Ok(fd)
    }

    pub(super) fn verified_dir_real(root_canon: &Path, parent: &Path) -> Result<PathBuf, String> {
        let fd = open_dir_verified(root_canon, parent)?;
        let real = super::super::fd_real_path_raw(fd.as_raw_fd())
            .ok_or("não foi possível revalidar o diretório de destino")?;
        Ok(std::fs::canonicalize(&real).unwrap_or(real))
    }

    /// Lê e hasheia o leaf via `openat` com `O_NOFOLLOW`: symlink → recusa
    /// (não escrevemos através de link); ausente → `None`; diretório → recusa.
    /// Devolve também o mode atual, para preservá-lo no save atômico.
    fn read_leaf(dir: &OwnedFd, leaf: &str) -> Result<Option<(String, u32)>, String> {
        match fs::openat(
            dir,
            leaf,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => {
                let stat = fs::fstat(&fd).map_err(|e| format!("stat falhou: {e}"))?;
                if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
                    return Err("destino não é um arquivo comum".into());
                }
                let mode = (stat.st_mode as u32) & 0o7777;
                let mut file = std::fs::File::from(fd);
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .map_err(|e| format!("falha ao ler conteúdo atual: {e}"))?;
                Ok(Some((hash_bytes(&buf), mode)))
            }
            Err(rustix::io::Errno::NOENT) => Ok(None),
            Err(rustix::io::Errno::LOOP) => {
                Err("destino é um symlink; não é editável pelo painel".into())
            }
            Err(e) => Err(format!("falha ao abrir destino: {e}")),
        }
    }

    pub(super) fn write_atomic<F: FnOnce(&Path)>(
        root_canon: &Path,
        parent: &Path,
        leaf: &str,
        content: &str,
        expected_hash: &str,
        before_rename: F,
    ) -> Result<WriteResult, String> {
        let dir = open_dir_verified(root_canon, parent)?;
        let baseline = read_leaf(&dir, leaf)?;
        let disk_hash = baseline.as_ref().map(|(h, _)| h.clone());
        let existing_mode = baseline.as_ref().map(|(_, m)| *m);
        let matches = match &disk_hash {
            Some(h) => h == expected_hash,
            None => expected_hash.is_empty(),
        };
        if !matches {
            return Ok(WriteResult::Conflict { disk_hash });
        }

        let tmp_name = format!(".tyba-write-{}", uuid::Uuid::new_v4());
        let tmp_fd = fs::openat(
            &dir,
            tmp_name.as_str(),
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|e| format!("falha ao criar temporário: {e}"))?;

        let write_result = (|| -> Result<(), String> {
            if let Some(mode) = existing_mode {
                fs::fchmod(&tmp_fd, Mode::from_raw_mode(mode as RawMode))
                    .map_err(|e| format!("falha ao aplicar permissões: {e}"))?;
            }
            let mut file = std::fs::File::from(tmp_fd);
            file.write_all(content.as_bytes())
                .map_err(|e| format!("falha ao escrever: {e}"))?;
            file.sync_all().map_err(|e| format!("falha no sync: {e}"))?;
            Ok(())
        })();
        if let Err(e) = write_result {
            let _ = fs::unlinkat(&dir, tmp_name.as_str(), AtFlags::empty());
            return Err(e);
        }

        before_rename(&parent.join(leaf));

        // CAS: reconfere o hash do disco imediatamente antes do rename. Mudou na
        // janela → o agente escreveu; aborta pra conflito sem promover o temp.
        let now = read_leaf(&dir, leaf)?.map(|(h, _)| h);
        if now != disk_hash {
            let _ = fs::unlinkat(&dir, tmp_name.as_str(), AtFlags::empty());
            return Ok(WriteResult::Conflict { disk_hash: now });
        }

        if let Err(e) = fs::renameat(&dir, tmp_name.as_str(), &dir, leaf) {
            let _ = fs::unlinkat(&dir, tmp_name.as_str(), AtFlags::empty());
            return Err(format!("falha ao promover temporário: {e}"));
        }
        Ok(WriteResult::Written {
            hash: hash_bytes(content.as_bytes()),
        })
    }

    pub(super) fn create(
        root_canon: &Path,
        parent: &Path,
        leaf: &str,
        is_dir: bool,
    ) -> Result<(), String> {
        let dir = open_dir_verified(root_canon, parent)?;
        if is_dir {
            fs::mkdirat(&dir, leaf, Mode::from_raw_mode(0o755)).map_err(|e| match e {
                rustix::io::Errno::EXIST => "já existe um item com esse nome".to_string(),
                other => format!("falha ao criar pasta: {other}"),
            })
        } else {
            let fd = fs::openat(
                &dir,
                leaf,
                OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o644),
            )
            .map_err(|e| match e {
                rustix::io::Errno::EXIST => "já existe um item com esse nome".to_string(),
                other => format!("falha ao criar arquivo: {other}"),
            })?;
            drop(fd);
            Ok(())
        }
    }

    pub(super) fn rename(
        root_canon: &Path,
        from_parent: &Path,
        from_leaf: &str,
        to_parent: &Path,
        to_leaf: &str,
    ) -> Result<(), String> {
        let from_dir = open_dir_verified(root_canon, from_parent)?;
        let to_dir = open_dir_verified(root_canon, to_parent)?;
        // Destino não pode existir (sem seguir symlink).
        match fs::statat(&to_dir, to_leaf, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return Err("já existe um item com esse nome".into()),
            Err(rustix::io::Errno::NOENT) => {}
            Err(e) => return Err(format!("falha ao checar destino: {e}")),
        }
        // renameat opera sobre a ENTRADA (o link, se for symlink), nunca segue
        // o alvo — mover um link que aponta pra fora move só o link.
        fs::renameat(&from_dir, from_leaf, &to_dir, to_leaf)
            .map_err(|e| format!("falha ao renomear: {e}"))
    }
}

#[cfg(not(unix))]
mod imp {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use super::hash_bytes;
    use super::WriteResult;

    // No Windows não há dirfd/openat portável aqui: re-canonicalizamos o pai
    // imediatamente antes da mutação e abortamos se escapar da raiz. Assimetria
    // consciente — sem preservação de mode POSIX nem O_NOFOLLOW.
    fn revalidate(root_canon: &Path, parent: &Path) -> Result<PathBuf, String> {
        let real = std::fs::canonicalize(parent)
            .map_err(|e| format!("diretório de destino inacessível: {e}"))?;
        if !real.starts_with(root_canon) {
            return Err("diretório de destino fora da raiz do painel".into());
        }
        Ok(real)
    }

    pub(super) fn verified_dir_real(root_canon: &Path, parent: &Path) -> Result<PathBuf, String> {
        revalidate(root_canon, parent)
    }

    fn read_hash(target: &Path) -> Result<Option<String>, String> {
        match std::fs::read(target) {
            Ok(bytes) => Ok(Some(hash_bytes(&bytes))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("falha ao ler conteúdo atual: {e}")),
        }
    }

    pub(super) fn write_atomic<F: FnOnce(&Path)>(
        root_canon: &Path,
        parent: &Path,
        leaf: &str,
        content: &str,
        expected_hash: &str,
        before_rename: F,
    ) -> Result<WriteResult, String> {
        let dir = revalidate(root_canon, parent)?;
        let target = dir.join(leaf);
        let disk_hash = read_hash(&target)?;
        let matches = match &disk_hash {
            Some(h) => h == expected_hash,
            None => expected_hash.is_empty(),
        };
        if !matches {
            return Ok(WriteResult::Conflict { disk_hash });
        }
        let tmp = dir.join(format!(".tyba-write-{}", uuid::Uuid::new_v4()));
        let write_result = (|| -> Result<(), String> {
            let mut file = std::fs::File::create(&tmp)
                .map_err(|e| format!("falha ao criar temporário: {e}"))?;
            file.write_all(content.as_bytes())
                .map_err(|e| format!("falha ao escrever: {e}"))?;
            file.sync_all().map_err(|e| format!("falha no sync: {e}"))?;
            Ok(())
        })();
        if let Err(e) = write_result {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        before_rename(&target);
        let now = read_hash(&target)?;
        if now != disk_hash {
            let _ = std::fs::remove_file(&tmp);
            return Ok(WriteResult::Conflict { disk_hash: now });
        }
        let _ = revalidate(root_canon, parent)?;
        if let Err(e) = std::fs::rename(&tmp, &target) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("falha ao promover temporário: {e}"));
        }
        Ok(WriteResult::Written {
            hash: hash_bytes(content.as_bytes()),
        })
    }

    pub(super) fn create(
        root_canon: &Path,
        parent: &Path,
        leaf: &str,
        is_dir: bool,
    ) -> Result<(), String> {
        let dir = revalidate(root_canon, parent)?;
        let target = dir.join(leaf);
        if target.exists() {
            return Err("já existe um item com esse nome".into());
        }
        if is_dir {
            std::fs::create_dir(&target).map_err(|e| format!("falha ao criar pasta: {e}"))
        } else {
            std::fs::File::create(&target)
                .map(|_| ())
                .map_err(|e| format!("falha ao criar arquivo: {e}"))
        }
    }

    pub(super) fn rename(
        root_canon: &Path,
        from_parent: &Path,
        from_leaf: &str,
        to_parent: &Path,
        to_leaf: &str,
    ) -> Result<(), String> {
        let from_dir = revalidate(root_canon, from_parent)?;
        let to_dir = revalidate(root_canon, to_parent)?;
        let from = from_dir.join(from_leaf);
        let to = to_dir.join(to_leaf);
        if to.exists() {
            return Err("já existe um item com esse nome".into());
        }
        std::fs::rename(&from, &to).map_err(|e| format!("falha ao renomear: {e}"))
    }
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

    #[test]
    fn write_refuses_content_over_the_edit_cap() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let huge = "z".repeat((EDIT_MAX_BYTES as usize) + 1);
        assert!(write_file(&root, "a.txt", &huge, &hash_bytes(b"x")).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_cas_refuses_a_racing_change_right_before_rename() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "v1").unwrap();
        let expected = hash_bytes(b"v1");
        let target = root.join("a.txt");
        let result = write_file_hooked(&root, "a.txt", "v2", &expected, |_| {
            std::fs::write(&target, "agent snuck in").unwrap();
        })
        .unwrap();
        match result {
            WriteResult::Conflict { disk_hash } => {
                assert_eq!(disk_hash, Some(hash_bytes(b"agent snuck in")));
            }
            other => panic!("esperava conflito de CAS, veio {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "agent snuck in",
            "o write do humano não pode ter sobrescrito o do agente na janela"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_preserves_existing_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp();
        for mode in [0o600u32, 0o755u32] {
            let name = format!("f{mode:o}.sh");
            std::fs::write(root.join(&name), "old").unwrap();
            std::fs::set_permissions(root.join(&name), std::fs::Permissions::from_mode(mode))
                .unwrap();
            write_file(&root, &name, "new", &hash_bytes(b"old")).unwrap();
            let got = std::fs::metadata(root.join(&name))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777;
            assert_eq!(got, mode, "o mode {mode:o} deve sobreviver ao save atômico");
        }
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
            err.contains("symlink"),
            "esperava recusa por symlink, veio {err}"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("secret.txt")).unwrap(),
            "outside",
            "o arquivo fora da raiz não pode ter sido escrito"
        );
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_refuses_a_swapped_intermediate_symlink_dir() {
        let root = tmp();
        let outside = tmp();
        std::fs::create_dir_all(outside.join("real")).unwrap();
        std::fs::write(outside.join("real/f.txt"), "outside").unwrap();
        // `sub` é um symlink pra fora; o pai canônico cai fora da raiz e o dirfd
        // revalidado recusa antes de escrever.
        std::os::unix::fs::symlink(outside.join("real"), root.join("sub")).unwrap();
        let err = write_file(&root, "sub/f.txt", "leak", "").unwrap_err();
        assert!(err.contains("fora") || err.contains("raiz"), "veio {err}");
        assert_eq!(
            std::fs::read_to_string(outside.join("real/f.txt")).unwrap(),
            "outside"
        );
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn create_rejects_traversal_and_root() {
        let root = tmp();
        assert!(create(&root, "../evil.txt", false).is_err());
        assert!(create(&root, "/etc/evil", false).is_err());
        assert!(create(&root, "", true).is_err());
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

    #[test]
    fn rename_rejects_empty_and_root() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        assert!(rename(&root, "", "b.txt").is_err());
        assert!(rename(&root, "a.txt", "").is_err());
        assert!(rename(&root, "a.txt", "/").is_err());
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
        assert!(err.contains("fora") || err.contains("raiz"), "veio {err}");
        assert!(root.join("a.txt").is_file(), "a origem não pode ter sumido");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn rename_of_escaping_symlink_moves_only_the_link() {
        let root = tmp();
        let outside = tmp();
        std::fs::write(outside.join("target.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(outside.join("target.txt"), root.join("link")).unwrap();
        rename(&root, "link", "renamed").unwrap();
        assert!(!root.join("link").exists());
        assert!(std::fs::symlink_metadata(root.join("renamed"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read_to_string(outside.join("target.txt")).unwrap(),
            "outside",
            "o alvo do symlink não pode ter sido tocado"
        );
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
    fn delete_rejects_escape_and_root() {
        let root = tmp();
        assert!(delete(&root, "../etc").is_err());
        assert!(delete(&root, "").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn delete_of_escaping_symlink_never_touches_the_target() {
        let root = tmp();
        let outside = tmp();
        std::fs::write(outside.join("keep.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(outside.join("keep.txt"), root.join("link")).unwrap();
        // Afirma a ESCOLHA do caminho, não o efeito de removê-lo: a garantia é
        // que o alvo do symlink nunca é o escolhido. Chamar a Lixeira de
        // verdade aqui provava menos — o resultado era descartado — e no macOS
        // abria um diálogo do Finder pedindo Touch ID a cada `cargo test`.
        let (_handle, target) = delete_target(&root, "link").unwrap();
        assert!(
            std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink(),
            "o escolhido é o próprio link, não o que ele resolve"
        );
        assert_ne!(
            target,
            outside.join("keep.txt"),
            "o alvo do symlink nunca vai pra Lixeira"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "outside"
        );
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }
}
