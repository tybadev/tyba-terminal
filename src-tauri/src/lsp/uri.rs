use std::path::{Path, PathBuf};

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};

const ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'|')
    .add(b'\\')
    .add(b'^');

pub fn from_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let normalized = raw.replace('\\', "/");
    let with_slash = if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{normalized}")
    };
    let encoded: String = with_slash
        .split('/')
        .map(|seg| utf8_percent_encode(seg, ENCODE_SET).to_string())
        .collect::<Vec<_>>()
        .join("/");
    format!("file://{encoded}")
}

pub fn to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode_str(rest).decode_utf8().ok()?;
    Some(PathBuf::from(decoded.into_owned()))
}

pub fn to_rel(root: &Path, uri: &str) -> Option<String> {
    let path = to_path(uri)?;
    let canon_path = std::fs::canonicalize(&path).unwrap_or(path);
    let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let stripped = canon_path.strip_prefix(&canon_root).ok()?;
    if stripped
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(stripped.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_plain_path() {
        let uri = from_path(Path::new("/private/wt/proj/src/main.rs"));
        assert_eq!(uri, "file:///private/wt/proj/src/main.rs");
        assert_eq!(
            to_path(&uri).unwrap(),
            PathBuf::from("/private/wt/proj/src/main.rs")
        );
    }

    #[test]
    fn encodes_spaces_and_specials() {
        let uri = from_path(Path::new("/a b/c#d.rs"));
        assert_eq!(uri, "file:///a%20b/c%23d.rs");
        assert_eq!(to_path(&uri).unwrap(), PathBuf::from("/a b/c#d.rs"));
    }

    #[test]
    fn keeps_utf8_segments_readable_after_decode() {
        let uri = from_path(Path::new("/café/αβγ.py"));
        assert_eq!(to_path(&uri).unwrap(), PathBuf::from("/café/αβγ.py"));
    }

    #[test]
    fn to_rel_rejects_a_parent_traversal_that_escapes_the_root() {
        let root = Path::new("/nonexistent-root/panel");
        let uri = from_path(Path::new("/nonexistent-root/panel/../secret.rs"));
        assert!(
            to_rel(root, &uri).is_none(),
            "um resultado com `..` não é in-root"
        );
    }

    #[test]
    fn to_rel_accepts_a_plain_child() {
        let root = Path::new("/nonexistent-root/panel");
        let uri = from_path(Path::new("/nonexistent-root/panel/src/main.rs"));
        assert_eq!(to_rel(root, &uri).as_deref(), Some("src/main.rs"));
    }
}
