//! Leitura e escrita em `~/.tyba/` tratando o disco como entrada não confiável.
//!
//! - teto de tamanho na leitura (o arquivo é lido por handle, nunca inteiro
//!   antes de checar);
//! - nada que, resolvido, aponte para fora da raiz — symlink dentro dela vale,
//!   para fora não;
//! - no Unix, arquivo e pasta precisam ser do usuário e não graváveis por grupo
//!   nem por outros;
//! - a pasta nasce `0700` e o arquivo `0600`, e a gravação é atômica (temp +
//!   rename), para que o watcher e um leitor concorrente nunca vejam meio
//!   arquivo.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn err(path: &Path, what: &str) -> String {
    format!("{}: {what}", path.display())
}

#[cfg(unix)]
fn check_owner(path: &Path, meta: &fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid não tem pré-condição nem falha.
    let me = unsafe { libc::geteuid() };
    if meta.uid() != me {
        return Err(err(path, "pertence a outro usuário; ignorado"));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(err(
            path,
            "pode ser alterado por outros usuários (grupo/outros com escrita); ignorado",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_owner(_path: &Path, _meta: &fs::Metadata) -> Result<(), String> {
    Ok(())
}

/// Cria a raiz (`0700`) se faltar e devolve o caminho canônico, já conferido.
pub fn ensure_root(root: &Path) -> Result<PathBuf, String> {
    if !root.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(root)
            .map_err(|e| err(root, &format!("não deu para criar: {e}")))?;
    }
    let canonical = fs::canonicalize(root).map_err(|e| err(root, &e.to_string()))?;
    let meta = fs::metadata(&canonical).map_err(|e| err(root, &e.to_string()))?;
    if !meta.is_dir() {
        return Err(err(root, "não é uma pasta"));
    }
    check_owner(root, &meta)?;
    Ok(canonical)
}

/// Resolve `path` e confere que continua dentro de `root`. `None` se não existe.
fn resolve_inside(root: &Path, path: &Path) -> Result<Option<PathBuf>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(err(path, &e.to_string())),
        Ok(_) => {}
    }
    let canonical_root = fs::canonicalize(root).map_err(|e| err(root, &e.to_string()))?;
    let canonical = match fs::canonicalize(path) {
        Ok(p) => p,
        // Symlink quebrado: existe, mas não aponta para nada legível.
        Err(_) => return Err(err(path, "link simbólico quebrado; ignorado")),
    };
    if !canonical.starts_with(&canonical_root) {
        return Err(err(path, "aponta para fora de ~/.tyba/; ignorado"));
    }
    Ok(Some(canonical))
}

fn open_nofollow(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    opts.open(path)
}

/// Lê `path` (dentro de `root`) com teto de `max` bytes. `Ok(None)` quando o
/// arquivo não existe.
pub fn read_capped(root: &Path, path: &Path, max: u64) -> Result<Option<String>, String> {
    let Some(target) = resolve_inside(root, path)? else {
        return Ok(None);
    };
    let file = open_nofollow(&target).map_err(|e| err(path, &e.to_string()))?;
    // Metadados do handle aberto, não do caminho: o que se confere é o que se lê.
    let meta = file.metadata().map_err(|e| err(path, &e.to_string()))?;
    if !meta.is_file() {
        return Err(err(path, "não é um arquivo comum; ignorado"));
    }
    check_owner(path, &meta)?;
    if meta.len() > max {
        return Err(err(
            path,
            &format!("maior que {} KiB; ignorado", max / 1024),
        ));
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| err(path, &e.to_string()))?;
    if bytes.len() as u64 > max {
        return Err(err(
            path,
            &format!("maior que {} KiB; ignorado", max / 1024),
        ));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| err(path, "não é texto UTF-8; ignorado"))
}

/// Grava `contents` em `path` de forma atômica, com `0600`. Se `path` é um
/// symlink dentro da raiz, grava no alvo — o link do usuário continua link.
pub fn write_atomic(root: &Path, path: &Path, contents: &str) -> Result<(), String> {
    let target = resolve_inside(root, path)?.unwrap_or_else(|| path.to_path_buf());
    let dir = target
        .parent()
        .ok_or_else(|| err(path, "sem pasta-mãe"))?
        .to_path_buf();
    let tmp = dir.join(format!(".config.toml.{}.tmp", uuid::Uuid::new_v4()));
    let write = || -> std::io::Result<()> {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &target)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        err(path, &format!("não deu para gravar: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".tyba");
        (dir, root)
    }

    #[test]
    fn missing_file_is_none() {
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        assert_eq!(
            read_capped(&root, &root.join("config.toml"), 1024).unwrap(),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_and_file_are_born_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        let path = root.join("config.toml");
        write_atomic(&root, &path, "a = 1\n").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(
            read_capped(&root, &path, 1024).unwrap().as_deref(),
            Some("a = 1\n")
        );
    }

    #[test]
    fn write_replaces_atomically_and_leaves_no_temp() {
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        let path = root.join("config.toml");
        write_atomic(&root, &path, "a = 1\n").unwrap();
        write_atomic(&root, &path, "a = 2\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "a = 2\n");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    #[test]
    fn oversized_file_is_refused() {
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        let path = root.join("config.toml");
        write_atomic(&root, &path, &"#".repeat(2048)).unwrap();
        let e = read_capped(&root, &path, 1024).unwrap_err();
        assert!(e.contains("maior que 1 KiB"), "{e}");
    }

    #[test]
    fn non_utf8_is_refused() {
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        let path = root.join("config.toml");
        write_atomic(&root, &path, "x").unwrap();
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(read_capped(&root, &path, 1024).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_out_of_root_is_refused() {
        let (d, root) = root();
        let root = ensure_root(&root).unwrap();
        let outside = d.path().join("fora.toml");
        fs::write(&outside, "a = 1\n").unwrap();
        let path = root.join("config.toml");
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        let e = read_capped(&root, &path, 1024).unwrap_err();
        assert!(e.contains("fora de ~/.tyba/"), "{e}");
        // e a escrita também não segue o link para fora
        assert!(write_atomic(&root, &path, "b = 2\n").is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "a = 1\n");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_inside_root_is_followed_and_kept() {
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        let real = root.join("real.toml");
        write_atomic(&root, &real, "a = 1\n").unwrap();
        let path = root.join("config.toml");
        std::os::unix::fs::symlink(&real, &path).unwrap();
        assert_eq!(
            read_capped(&root, &path, 1024).unwrap().as_deref(),
            Some("a = 1\n")
        );
        write_atomic(&root, &path, "a = 2\n").unwrap();
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "a = 2\n");
    }

    #[cfg(unix)]
    #[test]
    fn group_or_world_writable_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        let path = root.join("config.toml");
        write_atomic(&root, &path, "a = 1\n").unwrap();
        for mode in [0o620, 0o602] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            let e = read_capped(&root, &path, 1024).unwrap_err();
            assert!(e.contains("outros usuários"), "{e}");
        }
        // leitura por outros não é risco de escrita: 0644 passa
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_capped(&root, &path, 1024).is_ok());

        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(ensure_root(&root).is_err());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn directory_in_place_of_the_file_is_refused() {
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        let path = root.join("config.toml");
        fs::create_dir(&path).unwrap();
        assert!(read_capped(&root, &path, 1024).is_err());
    }
}
