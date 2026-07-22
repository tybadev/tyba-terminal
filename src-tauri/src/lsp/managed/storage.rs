use std::io::Read;
use std::path::{Component, Path, PathBuf};

use super::registry::{Archive, Managed, Pin, Source};

pub fn lsp_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("lsp")
}

pub fn server_dir(data_dir: &Path, id: &str) -> PathBuf {
    lsp_dir(data_dir).join(id)
}

pub fn version_dir(data_dir: &Path, id: &str, version: &str) -> PathBuf {
    server_dir(data_dir, id).join(version)
}

pub fn node_runtime_dir(data_dir: &Path, version: &str) -> PathBuf {
    lsp_dir(data_dir).join("runtime/node").join(version)
}

pub fn native_binary(data_dir: &Path, id: &str, pin: &Pin) -> PathBuf {
    version_dir(data_dir, id, pin.version).join(pin.member)
}

pub fn node_binary(data_dir: &Path, version: &str) -> PathBuf {
    node_runtime_dir(data_dir, version).join("bin/node")
}

pub fn server_entry_js(data_dir: &Path, managed: &Managed) -> Option<PathBuf> {
    let entry = managed.node_entry()?;
    Some(
        version_dir(data_dir, managed.id, managed.version())
            .join("node_modules")
            .join(entry),
    )
}

pub fn is_installed(data_dir: &Path, managed: &Managed) -> bool {
    match &managed.source {
        Source::Native { pins } => {
            let Some(current) = super::registry::Platform::current() else {
                return false;
            };
            match pins.iter().find(|p| p.platform == current) {
                Some(p) => native_binary(data_dir, managed.id, &p.pin).is_file(),
                None => false,
            }
        }
        Source::Node { .. } => {
            let entry_ok = server_entry_js(data_dir, managed)
                .map(|p| p.is_file())
                .unwrap_or(false);
            entry_ok && node_binary(data_dir, super::registry::node_runtime_version()).is_file()
        }
    }
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("storage: {e}"))?;
    }
    Ok(())
}

fn staging(final_dir: &Path) -> PathBuf {
    let nonce = uuid::Uuid::new_v4();
    let name = final_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    final_dir.with_file_name(format!(".staging-{name}-{nonce}"))
}

fn promote(stage: &Path, final_dir: &Path) -> Result<(), String> {
    ensure_parent(final_dir)?;
    if final_dir.exists() {
        let _ = std::fs::remove_dir_all(stage);
        return Ok(());
    }
    std::fs::rename(stage, final_dir).map_err(|e| {
        let _ = std::fs::remove_dir_all(stage);
        format!("storage: rename atômico falhou: {e}")
    })
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| format!("storage: {e}"))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| format!("storage: {e}"))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn safe_join(base: &Path, rel: &Path) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

pub fn extract_gzip_binary(src: &Path, dest_binary: &Path) -> Result<(), String> {
    ensure_parent(dest_binary)?;
    let file = std::fs::File::open(src).map_err(|e| format!("storage: {e}"))?;
    let mut decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let out = std::fs::File::create(dest_binary).map_err(|e| format!("storage: {e}"))?;
    let mut writer = std::io::BufWriter::new(out);
    std::io::copy(&mut decoder, &mut writer).map_err(|e| format!("storage: gunzip: {e}"))?;
    drop(writer);
    make_executable(dest_binary)
}

pub fn extract_raw_binary(src: &Path, dest_binary: &Path) -> Result<(), String> {
    ensure_parent(dest_binary)?;
    std::fs::copy(src, dest_binary).map_err(|e| format!("storage: {e}"))?;
    make_executable(dest_binary)
}

pub fn extract_zip(src: &Path, dest_dir: &Path, member: &str) -> Result<(), String> {
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("storage: {e}"))?;
    let file = std::fs::File::open(src).map_err(|e| format!("storage: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("storage: zip: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("storage: zip: {e}"))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err("storage: zip com caminho hostil (zip-slip)".into());
        };
        let Some(target) = safe_join(dest_dir, &rel) else {
            return Err("storage: zip com caminho hostil (zip-slip)".into());
        };
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| format!("storage: {e}"))?;
            continue;
        }
        ensure_parent(&target)?;
        let mut out = std::fs::File::create(&target).map_err(|e| format!("storage: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("storage: zip: {e}"))?;
    }
    let bin = dest_dir.join(member);
    if !bin.is_file() {
        return Err(format!("storage: zip sem o binário esperado {member}"));
    }
    make_executable(&bin)
}

fn untar_gz<R: Read>(reader: R, dest_dir: &Path, strip: usize) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(true);
    archive.set_overwrite(true);
    for entry in archive
        .entries()
        .map_err(|e| format!("storage: tar: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("storage: tar: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("storage: tar: {e}"))?
            .into_owned();
        let stripped: PathBuf = path.components().skip(strip).collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        let Some(target) = safe_join(dest_dir, &stripped) else {
            return Err("storage: tar com caminho hostil".into());
        };
        ensure_parent(&target)?;
        entry
            .unpack(&target)
            .map_err(|e| format!("storage: tar unpack: {e}"))?;
    }
    Ok(())
}

pub fn extract_tar_gz(src: &Path, dest_dir: &Path, strip: usize) -> Result<(), String> {
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("storage: {e}"))?;
    let file = std::fs::File::open(src).map_err(|e| format!("storage: {e}"))?;
    untar_gz(std::io::BufReader::new(file), dest_dir, strip)
}

pub fn extract_npm(src: &Path, node_modules: &Path, package: &str) -> Result<(), String> {
    let dest = node_modules.join(package);
    std::fs::create_dir_all(&dest).map_err(|e| format!("storage: {e}"))?;
    let file = std::fs::File::open(src).map_err(|e| format!("storage: {e}"))?;
    untar_gz(std::io::BufReader::new(file), &dest, 1)
}

pub fn install_native(
    data_dir: &Path,
    id: &str,
    pin: &Pin,
    verified_tmp: &Path,
) -> Result<PathBuf, String> {
    let final_dir = version_dir(data_dir, id, pin.version);
    let binary = final_dir.join(pin.member);
    if binary.is_file() {
        return Ok(binary);
    }
    let stage = staging(&final_dir);
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| format!("storage: {e}"))?;
    let staged_bin = stage.join(pin.member);
    let result = match pin.archive {
        Archive::Gzip => extract_gzip_binary(verified_tmp, &staged_bin),
        Archive::Raw => extract_raw_binary(verified_tmp, &staged_bin),
        Archive::Zip => extract_zip(verified_tmp, &stage, pin.member),
        Archive::TarGz => extract_tar_gz(verified_tmp, &stage, 0).and_then(|_| {
            if staged_bin.is_file() {
                Ok(())
            } else {
                Err(format!("storage: tar sem {}", pin.member))
            }
        }),
    };
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    promote(&stage, &final_dir)?;
    Ok(binary)
}

pub fn install_node_runtime(
    data_dir: &Path,
    pin: &Pin,
    verified_tmp: &Path,
) -> Result<PathBuf, String> {
    let final_dir = node_runtime_dir(data_dir, pin.version);
    let node = final_dir.join("bin/node");
    if node.is_file() {
        return Ok(node);
    }
    let stage = staging(&final_dir);
    let _ = std::fs::remove_dir_all(&stage);
    if let Err(e) = extract_tar_gz(verified_tmp, &stage, 1) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    if !stage.join("bin/node").is_file() {
        let _ = std::fs::remove_dir_all(&stage);
        return Err("storage: runtime node sem bin/node após extração".into());
    }
    promote(&stage, &final_dir)?;
    Ok(node)
}

pub fn install_node_server(
    data_dir: &Path,
    managed: &Managed,
    verified: &[(String, PathBuf)],
) -> Result<PathBuf, String> {
    let final_dir = version_dir(data_dir, managed.id, managed.version());
    let entry = server_entry_js(data_dir, managed).ok_or("storage: server não é de node")?;
    if entry.is_file() {
        return Ok(entry);
    }
    let stage = staging(&final_dir);
    let _ = std::fs::remove_dir_all(&stage);
    let node_modules = stage.join("node_modules");
    std::fs::create_dir_all(&node_modules).map_err(|e| format!("storage: {e}"))?;
    for (package, tmp) in verified {
        if let Err(e) = extract_npm(tmp, &node_modules, package) {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
    }
    let staged_entry = managed
        .node_entry()
        .map(|e| node_modules.join(e))
        .ok_or("storage: server sem entry")?;
    if !staged_entry.is_file() {
        let _ = std::fs::remove_dir_all(&stage);
        return Err("storage: entry do server não apareceu após extração".into());
    }
    promote(&stage, &final_dir)?;
    Ok(entry)
}

pub fn gc_previous_versions(data_dir: &Path, id: &str, keep_version: &str) {
    let dir = server_dir(data_dir, id);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == keep_version || name.starts_with(".staging-") {
            continue;
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

pub fn gc_node_runtime(data_dir: &Path, keep_version: &str) {
    let dir = lsp_dir(data_dir).join("runtime/node");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == keep_version || name.starts_with(".staging-") {
            continue;
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn gzip_bytes(data: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn layout_paths_are_versioned_under_lsp() {
        let d = Path::new("/data");
        assert_eq!(
            version_dir(d, "rust-analyzer", "1.0"),
            PathBuf::from("/data/lsp/rust-analyzer/1.0")
        );
        assert_eq!(
            node_runtime_dir(d, "24.18.0"),
            PathBuf::from("/data/lsp/runtime/node/24.18.0")
        );
    }

    #[test]
    fn gzip_install_lands_an_executable_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let data = b"#!/bin/sh\necho ra\n";
        let gz = tmp.path().join("ra.gz");
        std::fs::write(&gz, gzip_bytes(data)).unwrap();
        let pin = Pin {
            version: "1.0",
            url: "https://example/ra.gz",
            sha256: "x",
            size: 0,
            archive: Archive::Gzip,
            member: "rust-analyzer",
        };
        let bin = install_native(tmp.path(), "rust-analyzer", &pin, &gz).unwrap();
        assert_eq!(std::fs::read(&bin).unwrap(), data);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&bin).unwrap().permissions().mode();
            assert!(
                mode & 0o111 != 0,
                "binário instalado precisa ser executável"
            );
        }
    }

    #[test]
    fn install_is_idempotent_when_version_dir_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let gz = tmp.path().join("ra.gz");
        std::fs::write(&gz, gzip_bytes(b"one")).unwrap();
        let pin = Pin {
            version: "1.0",
            url: "u",
            sha256: "x",
            size: 0,
            archive: Archive::Gzip,
            member: "rust-analyzer",
        };
        let b1 = install_native(tmp.path(), "rust-analyzer", &pin, &gz).unwrap();
        let b2 = install_native(tmp.path(), "rust-analyzer", &pin, &gz).unwrap();
        assert_eq!(b1, b2);
        assert!(b1.is_file());
    }

    #[test]
    fn gc_removes_previous_versions_but_keeps_the_green_one() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::create_dir_all(version_dir(d, "taplo", "0.9")).unwrap();
        std::fs::create_dir_all(version_dir(d, "taplo", "0.10")).unwrap();
        std::fs::write(version_dir(d, "taplo", "0.9").join("taplo"), "old").unwrap();
        std::fs::write(version_dir(d, "taplo", "0.10").join("taplo"), "new").unwrap();
        gc_previous_versions(d, "taplo", "0.10");
        assert!(
            !version_dir(d, "taplo", "0.9").exists(),
            "versão anterior sai"
        );
        assert!(
            version_dir(d, "taplo", "0.10").join("taplo").is_file(),
            "a versão verde permanece"
        );
    }

    #[test]
    fn gc_never_touches_the_previous_version_before_a_green_boot() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::create_dir_all(version_dir(d, "taplo", "0.9")).unwrap();
        std::fs::create_dir_all(version_dir(d, "taplo", "0.10")).unwrap();
        assert!(version_dir(d, "taplo", "0.9").exists());
        assert!(version_dir(d, "taplo", "0.10").exists());
    }

    #[test]
    fn gc_ignores_in_flight_staging_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let server = server_dir(d, "taplo");
        std::fs::create_dir_all(server.join("0.10")).unwrap();
        std::fs::create_dir_all(server.join(".staging-0.11-abc")).unwrap();
        gc_previous_versions(d, "taplo", "0.10");
        assert!(
            server.join(".staging-0.11-abc").exists(),
            "staging em voo não é lixo"
        );
    }

    #[test]
    fn zip_extraction_rejects_zip_slip() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("evil.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(file);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            w.start_file("../escape.txt", opts).unwrap();
            w.write_all(b"pwned").unwrap();
            w.finish().unwrap();
        }
        let dest = tmp.path().join("out");
        let err = extract_zip(&zip_path, &dest, "whatever").unwrap_err();
        assert!(err.contains("zip-slip") || err.contains("binário esperado"));
        assert!(!tmp.path().join("escape.txt").exists());
    }

    #[test]
    fn zip_install_places_the_member_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("tfls.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(file);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            w.start_file("LICENSE.txt", opts).unwrap();
            w.write_all(b"license").unwrap();
            w.start_file("terraform-ls", opts).unwrap();
            w.write_all(b"#!/bin/sh\n").unwrap();
            w.finish().unwrap();
        }
        let pin = Pin {
            version: "0.38.8",
            url: "u",
            sha256: "x",
            size: 0,
            archive: Archive::Zip,
            member: "terraform-ls",
        };
        let bin = install_native(tmp.path(), "terraform-ls", &pin, &zip_path).unwrap();
        assert!(bin.is_file());
        assert!(bin.ends_with("terraform-ls"));
    }
}
