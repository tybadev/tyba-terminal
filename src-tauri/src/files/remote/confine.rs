use super::fs::{RemoteError, RemoteFs, RemoteResult};

/// Normaliza um caminho relativo vindo do webview em segmentos seguros. Recusa
/// caminho absoluto, `..` (traversal) e caracteres de controle/NUL. Aceita `.`
/// (descartado). Devolve o rel normalizado com `/` como separador, sem barra
/// inicial. Mesma postura da `resolve_within` local, adaptada a string remota.
pub fn validate_rel(rel: &str) -> RemoteResult<String> {
    let trimmed = rel.trim_start_matches('/');
    if trimmed.contains('\0') {
        return Err(RemoteError::Denied);
    }
    let mut out: Vec<&str> = Vec::new();
    for seg in trimmed.split('/') {
        match seg {
            "" | "." => {}
            ".." => return Err(RemoteError::Denied),
            _ => {
                if seg.contains('\\') {
                    return Err(RemoteError::Denied);
                }
                out.push(seg);
            }
        }
    }
    Ok(out.join("/"))
}

/// Um nome de arquivo/pasta isolado (leaf): sem separador, sem `.`/`..`, sem NUL.
pub fn valid_leaf(name: &str) -> RemoteResult<()> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(RemoteError::Denied);
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(RemoteError::Denied);
    }
    Ok(())
}

/// Separa um rel normalizado em (pai, leaf). Recusa a raiz vazia.
pub fn split_leaf(rel: &str) -> RemoteResult<(String, String)> {
    let norm = validate_rel(rel)?;
    if norm.is_empty() {
        return Err(RemoteError::Denied);
    }
    match norm.rsplit_once('/') {
        Some((parent, leaf)) => {
            valid_leaf(leaf)?;
            Ok((parent.to_string(), leaf.to_string()))
        }
        None => {
            valid_leaf(&norm)?;
            Ok((String::new(), norm))
        }
    }
}

pub fn join(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_string()
    } else if root.ends_with('/') {
        format!("{root}{rel}")
    } else {
        format!("{root}/{rel}")
    }
}

/// `candidate` (já canônico via realpath) está dentro de `root` (também canônico)?
pub fn within(root: &str, candidate: &str) -> bool {
    if candidate == root {
        return true;
    }
    let prefix = if root.ends_with('/') {
        root.to_string()
    } else {
        format!("{root}/")
    };
    candidate.starts_with(&prefix)
}

/// Resolve `rel` (existente) contra a raiz e confirma, pelo realpath do host,
/// que o alvo canônico continua dentro da raiz — fecha symlink que escapa.
pub fn confine_existing(fs: &dyn RemoteFs, root_real: &str, rel: &str) -> RemoteResult<String> {
    let norm = validate_rel(rel)?;
    let candidate = join(root_real, &norm);
    let real = fs.realpath(&candidate)?;
    if !within(root_real, &real) {
        return Err(RemoteError::Denied);
    }
    Ok(real)
}

/// Resolve o diretório-pai (existente) de uma operação de escrita e confirma o
/// confinamento por realpath. Devolve o caminho real do pai — o leaf é anexado
/// pelo chamador sem novo realpath (o leaf pode não existir ainda).
pub fn confine_parent(
    fs: &dyn RemoteFs,
    root_real: &str,
    parent_rel: &str,
) -> RemoteResult<String> {
    let norm = validate_rel(parent_rel)?;
    let candidate = join(root_real, &norm);
    let real = fs.realpath(&candidate)?;
    if !within(root_real, &real) {
        return Err(RemoteError::Denied);
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rel_strips_dot_and_leading_slash() {
        assert_eq!(validate_rel("/a/./b").unwrap(), "a/b");
        assert_eq!(validate_rel("").unwrap(), "");
        assert_eq!(validate_rel("src//lib.rs").unwrap(), "src/lib.rs");
    }

    #[test]
    fn validate_rel_rejects_traversal_and_nul_and_backslash() {
        assert!(validate_rel("../etc/passwd").is_err());
        assert!(validate_rel("a/../../b").is_err());
        assert!(validate_rel("a/\0/b").is_err());
        assert!(validate_rel("a\\b").is_err());
    }

    #[test]
    fn split_leaf_separates_parent_and_leaf() {
        assert_eq!(
            split_leaf("src/lib.rs").unwrap(),
            ("src".to_string(), "lib.rs".to_string())
        );
        assert_eq!(
            split_leaf("top.txt").unwrap(),
            (String::new(), "top.txt".to_string())
        );
        assert!(split_leaf("").is_err());
        assert!(split_leaf("/").is_err());
    }

    #[test]
    fn within_holds_only_for_paths_under_root() {
        assert!(within("/home/u/proj", "/home/u/proj"));
        assert!(within("/home/u/proj", "/home/u/proj/src/a.rs"));
        assert!(!within("/home/u/proj", "/home/u/proj-evil"));
        assert!(!within("/home/u/proj", "/etc/passwd"));
        assert!(!within("/home/u/proj", "/home/u"));
    }

    #[test]
    fn join_handles_root_with_and_without_slash() {
        assert_eq!(join("/root", "a/b"), "/root/a/b");
        assert_eq!(join("/root/", "a"), "/root/a");
        assert_eq!(join("/root", ""), "/root");
    }
}
