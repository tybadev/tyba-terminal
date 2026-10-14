//! Leitura e escrita em `~/.tyba/` tratando o disco como entrada não confiável.
//!
//! - teto de tamanho na leitura (o arquivo é lido por handle, nunca inteiro
//!   antes de checar);
//! - symlink: nos temas, só dentro da raiz; no `config.toml` (e na própria
//!   raiz), também para fora — o estilo stow/chezmoi de dotfiles —, desde que o
//!   alvo final seja arquivo comum do usuário, sem escrita de grupo/outros
//!   ([`Links`]);
//! - no Unix, a raiz e o arquivo precisam ser do usuário e não graváveis por grupo
//!   nem por outros;
//! - a pasta nasce `0700` e o arquivo `0600`, e a gravação é atômica (temp +
//!   rename), para que o watcher e um leitor concorrente nunca vejam meio
//!   arquivo;
//! - a gravação substitui só o arquivo que foi lido ([`Expect`]): um link
//!   trocado entre a leitura e a gravação não a redireciona.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn err(path: &Path, what: &str) -> String {
    format!("{}: {what}", path.display())
}

#[cfg(unix)]
fn check_owner(path: &Path, meta: &fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    check_owner_ids(path, meta.uid(), meta.mode())
}

#[cfg(unix)]
fn check_owner_ids(path: &Path, uid: u32, mode: u32) -> Result<(), String> {
    // SAFETY: geteuid não tem pré-condição nem falha.
    let me = unsafe { libc::geteuid() };
    if uid != me {
        return Err(err(path, "pertence a outro usuário; ignorado"));
    }
    if mode & 0o022 != 0 {
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

/// Confere a raiz que já existe (pasta, do usuário, sem escrita de outros) e
/// devolve o caminho canônico. Não cria nada.
pub fn check_root(root: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(root).map_err(|e| err(root, &e.to_string()))?;
    let meta = fs::metadata(&canonical).map_err(|e| err(root, &e.to_string()))?;
    if !meta.is_dir() {
        return Err(err(root, "não é uma pasta"));
    }
    check_owner(root, &meta)?;
    Ok(canonical)
}

/// Cria `dir` (e o que faltar acima) com `0700`. Já existente: nada muda.
pub fn create_dir_private(dir: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Cria a raiz (`0700`) se faltar e devolve o caminho canônico, já conferido.
pub fn ensure_root(root: &Path) -> Result<PathBuf, String> {
    if !root.exists() {
        create_dir_private(root).map_err(|e| err(root, &format!("não deu para criar: {e}")))?;
    }
    check_root(root)
}

/// Identidade do arquivo lido (`dev`, `ino`): a gravação só substitui esse
/// mesmo arquivo, nunca outro que um link trocado entre a leitura e a gravação
/// apontaria.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
pub struct FileId {
    dev: u64,
    ino: u64,
}

#[cfg(unix)]
fn file_id(meta: &fs::Metadata) -> FileId {
    use std::os::unix::fs::MetadataExt;
    FileId {
        dev: meta.dev(),
        ino: meta.ino(),
    }
}

#[cfg(not(unix))]
fn file_id(_meta: &fs::Metadata) -> FileId {
    FileId { dev: 0, ino: 0 }
}

/// O que a gravação espera encontrar no lugar do arquivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// O arquivo que foi lido (existia na leitura).
    Same(FileId),
    /// Nada: o arquivo não existia na leitura e tem de continuar não existindo.
    Absent,
    /// Qualquer arquivo comum do usuário (temas, sempre dentro da raiz).
    Any,
}

/// Até onde um symlink pode levar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Links {
    /// Só para dentro da raiz (pasta de temas).
    InsideRoot,
    /// Para qualquer lugar, se o alvo passar na conferência de dono e
    /// permissão (o `config.toml` versionado num repositório de dotfiles).
    OwnedTarget,
}

/// Resolve `path`; com [`Links::InsideRoot`], confere que continua dentro de
/// `root`. `None` se não existe.
fn resolve(root: &Path, path: &Path, links: Links) -> Result<Option<PathBuf>, String> {
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
    if links == Links::InsideRoot && !canonical.starts_with(&canonical_root) {
        return Err(err(path, "aponta para fora de ~/.tyba/; ignorado"));
    }
    Ok(Some(canonical))
}

/// `true` se a própria raiz é symlink ou `path` resolve para fora dela — o
/// `config.toml` de dotfiles, que um agente na jaula pode alcançar (e editar)
/// por um caminho que a jaula não esconde.
pub fn linked_out_of_root(root: &Path, path: &Path) -> bool {
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink()) {
        return true;
    }
    let Ok(canonical_root) = fs::canonicalize(root) else {
        return false;
    };
    fs::canonicalize(path).is_ok_and(|p| !p.starts_with(&canonical_root))
}

/// Caminho final de `path` (o alvo, se for symlink) — é o que se vigia e onde
/// se grava. `None` se não existe.
pub fn target(root: &Path, path: &Path, links: Links) -> Result<Option<PathBuf>, String> {
    resolve(root, path, links)
}

fn open_nofollow(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK: um FIFO no lugar do arquivo travaria a abertura à espera
        // de quem escreva. Em arquivo comum não muda nada.
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    opts.open(path)
}

/// Lê `path` (dentro de `root`, symlink só para dentro) com teto de `max`
/// bytes. `Ok(None)` quando o arquivo não existe.
pub fn read_capped(root: &Path, path: &Path, max: u64) -> Result<Option<String>, String> {
    Ok(read_with(root, path, max, Links::InsideRoot)?.map(|(text, _)| text))
}

/// Como [`read_capped`], devolvendo também a identidade do arquivo lido.
pub fn read_capped_with_id(
    root: &Path,
    path: &Path,
    max: u64,
) -> Result<Option<(String, FileId)>, String> {
    read_with(root, path, max, Links::InsideRoot)
}

/// Identidade da entrada `path` em si (sem seguir symlink). `None` se não
/// existe. Fora do Unix não há inode: só serve para comparar no Unix.
#[cfg(unix)]
pub fn entry_id(path: &Path) -> Option<FileId> {
    fs::symlink_metadata(path).ok().map(|m| file_id(&m))
}

/// Como [`read_capped`], aceitando symlink para fora com alvo do usuário.
pub fn read_linked(root: &Path, path: &Path, max: u64) -> Result<Option<String>, String> {
    Ok(read_linked_with_id(root, path, max)?.map(|(text, _)| text))
}

/// Como [`read_linked`], devolvendo também a identidade do arquivo lido — para
/// a gravação seguinte ([`write_linked`]) substituir só ele.
pub fn read_linked_with_id(
    root: &Path,
    path: &Path,
    max: u64,
) -> Result<Option<(String, FileId)>, String> {
    read_with(root, path, max, Links::OwnedTarget)
}

fn read_with(
    root: &Path,
    path: &Path,
    max: u64,
    links: Links,
) -> Result<Option<(String, FileId)>, String> {
    let Some(target) = resolve(root, path, links)? else {
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
    let id = file_id(&meta);
    String::from_utf8(bytes)
        .map(|text| Some((text, id)))
        .map_err(|_| err(path, "não é texto UTF-8; ignorado"))
}

/// Grava `contents` em `path` de forma atômica, com `0600`. Se `path` é um
/// symlink (dentro da raiz), grava no alvo — o link do usuário continua link.
pub fn write_atomic(root: &Path, path: &Path, contents: &str) -> Result<(), String> {
    write_with(root, path, contents, Links::InsideRoot, Expect::Any)
}

/// Como [`write_atomic`], seguindo symlink para fora com alvo do usuário, e só
/// se no lugar ainda estiver o que `expect` diz (o que a leitura viu).
pub fn write_linked(
    root: &Path,
    path: &Path,
    contents: &str,
    expect: Expect,
) -> Result<(), String> {
    write_with(root, path, contents, Links::OwnedTarget, expect)
}

fn write_with(
    root: &Path,
    path: &Path,
    contents: &str,
    links: Links,
    expect: Expect,
) -> Result<(), String> {
    let target = resolve(root, path, links)?.unwrap_or_else(|| path.to_path_buf());
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(err(path, "sem pasta-mãe"));
    };
    replace_in_dir(path, dir, name, contents, expect)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // os tipos do `stat` variam por SO
fn stat_fields(st: &rustix::fs::Stat) -> (FileId, u32, u32) {
    let id = FileId {
        dev: st.st_dev as u64,
        ino: st.st_ino as u64,
    };
    (id, st.st_uid as u32, st.st_mode as u32)
}

/// Tudo pelo handle da pasta: o que se confere (`statat`) é a entrada que o
/// `renameat` substitui, sem um caminho que um link trocado redirecione. A
/// temp nasce ao lado do alvo (o rename precisa do mesmo disco).
#[cfg(unix)]
fn replace_in_dir(
    path: &Path,
    dir: &Path,
    name: &std::ffi::OsStr,
    contents: &str,
    expect: Expect,
) -> Result<(), String> {
    use rustix::fs::{self as rfs, AtFlags, FileType, Mode, OFlags};
    let changed = || err(path, "o arquivo mudou durante a gravação; nada foi gravado");
    let dirfd = rfs::open(
        dir,
        OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| err(dir, &e.to_string()))?;
    match rfs::statat(&dirfd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(st) => {
            let (id, uid, mode) = stat_fields(&st);
            match expect {
                Expect::Same(want) if want == id => {}
                Expect::Any => {}
                _ => return Err(changed()),
            }
            // O que vai ser substituído: arquivo comum do usuário. FIFO, device
            // ou arquivo alheio no alvo do link não viram vítima do rename.
            if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
                return Err(err(path, "não é um arquivo comum; não gravei"));
            }
            check_owner_ids(path, uid, mode)?;
        }
        Err(rustix::io::Errno::NOENT) => {
            if let Expect::Same(_) = expect {
                return Err(changed());
            }
        }
        Err(e) => return Err(err(path, &e.to_string())),
    }
    let tmp = format!(".tyba-{}.tmp", uuid::Uuid::new_v4());
    let fd = rfs::openat(
        &dirfd,
        tmp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|e| err(path, &format!("não deu para gravar: {e}")))?;
    let write = || -> std::io::Result<()> {
        let mut file = File::from(fd);
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        rfs::renameat(&dirfd, tmp.as_str(), &dirfd, name)?;
        Ok(())
    };
    write().map_err(|e| {
        let _ = rfs::unlinkat(&dirfd, tmp.as_str(), AtFlags::empty());
        err(path, &format!("não deu para gravar: {e}"))
    })
}

#[cfg(not(unix))]
fn replace_in_dir(
    path: &Path,
    dir: &Path,
    name: &std::ffi::OsStr,
    contents: &str,
    _expect: Expect,
) -> Result<(), String> {
    let target = dir.join(name);
    if let Ok(meta) = fs::metadata(&target) {
        if !meta.is_file() {
            return Err(err(path, "não é um arquivo comum; não gravei"));
        }
    }
    let tmp = dir.join(format!(".tyba-{}.tmp", uuid::Uuid::new_v4()));
    let write = || -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
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
    fn dotfiles_symlink_out_of_root_is_accepted_when_owned_and_private() {
        use std::os::unix::fs::PermissionsExt;
        let (d, root) = root();
        let root = ensure_root(&root).unwrap();
        let dotfiles = d.path().join("dotfiles");
        fs::create_dir(&dotfiles).unwrap();
        fs::set_permissions(&dotfiles, fs::Permissions::from_mode(0o755)).unwrap();
        let real = dotfiles.join("tyba.toml");
        fs::write(&real, "a = 1\n").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o644)).unwrap();
        let path = root.join("config.toml");
        std::os::unix::fs::symlink(&real, &path).unwrap();

        assert_eq!(
            read_linked(&root, &path, 1024).unwrap().as_deref(),
            Some("a = 1\n")
        );
        assert_eq!(
            target(&root, &path, Links::OwnedTarget).unwrap(),
            Some(fs::canonicalize(&real).unwrap())
        );
        write_linked(&root, &path, "a = 2\n", Expect::Any).unwrap();
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "a = 2\n");
        // nenhuma temp sobrando ao lado do alvo
        assert_eq!(fs::read_dir(&dotfiles).unwrap().count(), 1);

        // alvo gravável por outros: não
        fs::set_permissions(&real, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(read_linked(&root, &path, 1024).is_err());
        assert!(write_linked(&root, &path, "x", Expect::Any).is_err());
        assert_eq!(fs::read_to_string(&real).unwrap(), "a = 2\n");
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn group_writable_target_dir_does_not_block_the_write() {
        // repositório de dotfiles (ou pasta de temas) sob umask 002
        use std::os::unix::fs::PermissionsExt;
        let (d, root) = root();
        let root = ensure_root(&root).unwrap();
        let dotfiles = d.path().join("dotfiles");
        fs::create_dir(&dotfiles).unwrap();
        fs::set_permissions(&dotfiles, fs::Permissions::from_mode(0o775)).unwrap();
        let real = dotfiles.join("tyba.toml");
        fs::write(&real, "a = 1\n").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o644)).unwrap();
        let path = root.join("config.toml");
        std::os::unix::fs::symlink(&real, &path).unwrap();
        write_linked(&root, &path, "a = 2\n", Expect::Any).unwrap();
        assert_eq!(fs::read_to_string(&real).unwrap(), "a = 2\n");
    }

    #[cfg(unix)]
    #[test]
    fn private_dir_is_born_0700() {
        use std::os::unix::fs::PermissionsExt;
        let (d, _) = root();
        let dir = d.path().join("a/themes");
        create_dir_private(&dir).unwrap();
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn write_replaces_only_the_file_that_was_read() {
        let (d, root) = root();
        let root = ensure_root(&root).unwrap();
        let a = d.path().join("a.toml");
        let b = d.path().join("b.toml");
        fs::write(&a, "a = 1\n").unwrap();
        fs::write(&b, "b = 1\n").unwrap();
        let path = root.join("config.toml");
        std::os::unix::fs::symlink(&a, &path).unwrap();

        // leitura e gravação do mesmo arquivo: passa
        let (_, id) = read_linked_with_id(&root, &path, 1024).unwrap().unwrap();
        write_linked(&root, &path, "a = 2\n", Expect::Same(id)).unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "a = 2\n");

        // o link troca de alvo entre a leitura e a gravação: nada é gravado
        let (_, id) = read_linked_with_id(&root, &path, 1024).unwrap().unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&b, &path).unwrap();
        let e = write_linked(&root, &path, "x = 1\n", Expect::Same(id)).unwrap_err();
        assert!(e.contains("mudou durante a gravação"), "{e}");
        assert_eq!(fs::read_to_string(&a).unwrap(), "a = 2\n");
        assert_eq!(fs::read_to_string(&b).unwrap(), "b = 1\n");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 3);

        // o arquivo lido sumiu: também não
        fs::remove_file(&path).unwrap();
        assert!(write_linked(&root, &path, "x = 1\n", Expect::Same(id)).is_err());
        assert!(fs::symlink_metadata(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn absent_file_is_created_only_if_still_absent() {
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        let path = root.join("config.toml");
        assert_eq!(read_linked_with_id(&root, &path, 1024).unwrap(), None);
        write_linked(&root, &path, "a = 1\n", Expect::Absent).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "a = 1\n");

        // alguém criou o arquivo depois da leitura (que não achou nada): não
        let e = write_linked(&root, &path, "a = 2\n", Expect::Absent).unwrap_err();
        assert!(e.contains("mudou durante a gravação"), "{e}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "a = 1\n");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn dotfiles_symlink_to_a_fifo_is_refused_without_blocking() {
        let (d, root) = root();
        let root = ensure_root(&root).unwrap();
        let fifo = d.path().join("fifo");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: caminho válido, terminado em NUL.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let path = root.join("config.toml");
        std::os::unix::fs::symlink(&fifo, &path).unwrap();
        assert!(read_linked(&root, &path, 1024).is_err());
        assert!(write_linked(&root, &path, "x", Expect::Any).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_root_is_accepted_when_private() {
        let (d, root) = root();
        let real = d.path().join("dotfiles-tyba");
        ensure_root(&real).unwrap();
        std::os::unix::fs::symlink(&real, &root).unwrap();
        let canonical = ensure_root(&root).unwrap();
        assert_eq!(canonical, fs::canonicalize(&real).unwrap());
        write_linked(&root, &root.join("config.toml"), "a = 1\n", Expect::Any).unwrap();
        assert!(real.join("config.toml").is_file());
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

    #[cfg(unix)]
    #[test]
    fn fifo_in_place_of_the_file_does_not_hang() {
        use std::os::unix::ffi::OsStrExt;
        let (_d, root) = root();
        let root = ensure_root(&root).unwrap();
        let path = root.join("config.toml");
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: caminho válido terminado em NUL.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let (r, p) = (root.clone(), path.clone());
        std::thread::spawn(move || {
            let _ = tx.send(read_capped(&r, &p, 1024));
        });
        let res = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("read_capped travou num FIFO");
        assert!(res.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn check_root_never_creates() {
        let (_d, root) = root();
        assert!(check_root(&root).is_err());
        assert!(!root.exists());
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
