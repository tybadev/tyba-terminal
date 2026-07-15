use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    Literal(PathBuf),
    Subpath(PathBuf),
    Node(PathBuf),
    Prefix(PathBuf),
    Family(PathBuf),
}

#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    pub allow: Vec<Rule>,
    pub except: Vec<Rule>,
}

impl Rule {
    pub fn path(&self) -> &Path {
        match self {
            Rule::Literal(p)
            | Rule::Subpath(p)
            | Rule::Node(p)
            | Rule::Prefix(p)
            | Rule::Family(p) => p,
        }
    }
}

impl RuleSet {
    pub fn allow(rules: Vec<Rule>) -> Self {
        RuleSet {
            allow: rules,
            except: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AgentAccess {
    pub read: Vec<RuleSet>,
    pub write: Vec<RuleSet>,
}

pub fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

pub fn escape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\^$.|?*+()[]{}\"".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn canonical_form(path: &Path) -> Option<PathBuf> {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return Some(canon);
    }
    let parent = path.parent()?;
    let name = path.file_name()?;
    let canon_parent = std::fs::canonicalize(parent).ok()?;
    Some(canon_parent.join(name))
}

pub fn variants(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![path.to_path_buf()];
    if let Some(canon) = canonical_form(path) {
        if !out.contains(&canon) {
            out.push(canon);
        }
    }
    for base in out.clone() {
        let s = path_str(&base);
        if let Some(rest) = s.strip_prefix("/System/Volumes/Data/") {
            let bare = PathBuf::from(format!("/{rest}"));
            if !out.contains(&bare) {
                out.push(bare);
            }
        } else if !s.starts_with("/System/") && !s.starts_with("/private/") {
            let firmlinked = PathBuf::from(format!("/System/Volumes/Data{s}"));
            if !out.contains(&firmlinked) {
                out.push(firmlinked);
            }
        }
    }
    out
}

fn render_one(rule: &Rule, path: &Path) -> String {
    let s = path_str(path);
    match rule {
        Rule::Literal(_) => format!("(literal \"{}\")", escape_string(&s)),
        Rule::Subpath(_) => format!("(subpath \"{}\")", escape_string(&s)),
        Rule::Node(_) => format!(
            "(literal \"{}\") (subpath \"{}\") (regex #\"^{}(/.*)?$\")",
            escape_string(&s),
            escape_string(&s),
            escape_regex(&s),
        ),
        Rule::Prefix(_) => format!("(regex #\"^{}\")", escape_regex(&s)),
        Rule::Family(_) => format!("(regex #\"^{}($|[-.~])\")", escape_regex(&s)),
    }
}

pub fn render_rule(rule: &Rule) -> String {
    variants(rule.path())
        .iter()
        .map(|v| render_one(rule, v))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn render_rules(rules: &[Rule]) -> String {
    rules.iter().map(render_rule).collect::<Vec<_>>().join(" ")
}

pub fn render_ruleset(op: &str, set: &RuleSet) -> Option<String> {
    if set.allow.is_empty() {
        return None;
    }
    let allow = render_rules(&set.allow);
    if set.except.is_empty() {
        return Some(format!("(allow {op} {allow})"));
    }
    let except = render_rules(&set.except);
    Some(format!(
        "(allow {op} (require-all (require-any {allow}) (require-not (require-any {except}))))"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_string_quotes_and_backslashes() {
        assert_eq!(escape_string(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(escape_string("/plain/path"), "/plain/path");
    }

    #[test]
    fn escape_regex_neutralizes_metachars() {
        assert_eq!(escape_regex("/a.b/c+d"), r"/a\.b/c\+d");
        assert_eq!(escape_regex("(x)[y]{z}^$|?*"), r"\(x\)\[y\]\{z\}\^\$\|\?\*");
        assert_eq!(escape_regex(r#"/a"b"#), r#"/a\"b"#);
    }

    #[test]
    fn regex_body_is_never_double_escaped() {
        let rendered = render_rule(&Rule::Node(PathBuf::from("/nonexistent-d/.git")));
        assert!(rendered.contains(r"\.git"));
        assert!(
            !rendered.contains(r"\\.git"),
            "barra dupla vira 'backslash literal' no motor de regex: a regra é aceita pelo \
             sandbox-exec e não casa com nada — protege no papel, não na máquina"
        );
    }

    #[test]
    fn node_rule_emits_literal_subpath_and_anchored_regex() {
        let rendered = render_rule(&Rule::Node(PathBuf::from("/nonexistent-x/.git")));
        assert!(rendered.contains(r#"(literal "/nonexistent-x/.git")"#));
        assert!(rendered.contains(r#"(subpath "/nonexistent-x/.git")"#));
        assert!(rendered.contains(r#"(regex #"^/nonexistent-x/\.git(/.*)?$")"#));
    }

    #[test]
    fn prefix_rule_emits_unanchored_tail_regex() {
        let rendered = render_rule(&Rule::Prefix(PathBuf::from("/nonexistent-h/.claude.json")));
        assert!(rendered.contains(r#"(regex #"^/nonexistent-h/\.claude\.json")"#));
    }

    #[test]
    fn family_rule_matches_file_and_dotted_siblings_not_bare_suffix() {
        let rendered = render_rule(&Rule::Family(PathBuf::from("/nonexistent-f/.claude.json")));
        assert!(rendered.contains(r#"(regex #"^/nonexistent-f/\.claude\.json($|[-.~])")"#));
    }

    #[test]
    fn canonical_form_falls_back_to_parent_for_absent_leaf() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap();
        let absent = dir.path().join("does-not-exist-yet");
        let vs = variants(&absent);
        assert!(
            vs.contains(&real.join("does-not-exist-yet")),
            "sem a forma canônica do pai, um except/allow sobre um path ainda inexistente \
             (socket de hook, prefixo tyba-) não casa o path resolvido pelo kernel"
        );
    }

    #[cfg(unix)]
    #[test]
    fn variants_resolve_symlinks_preserving_literal() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let vs = variants(&link);
        assert!(vs.contains(&link));
        assert!(vs.contains(&target.canonicalize().unwrap()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn variants_add_firmlink_form_for_data_volume_paths() {
        let vs = variants(Path::new("/nonexistent-y/dir"));
        assert!(vs.contains(&PathBuf::from("/nonexistent-y/dir")));
        assert!(vs.contains(&PathBuf::from("/System/Volumes/Data/nonexistent-y/dir")));
    }

    #[test]
    fn variants_strip_firmlink_prefix_back_to_bare_path() {
        let vs = variants(Path::new("/System/Volumes/Data/nonexistent-z"));
        assert!(vs.contains(&PathBuf::from("/nonexistent-z")));
    }

    #[test]
    fn variants_never_firmlink_system_or_private() {
        for p in ["/System/Library", "/private/tmp/x"] {
            let vs = variants(Path::new(p));
            assert!(vs.iter().all(|v| !v.starts_with("/System/Volumes/Data")));
        }
    }

    #[test]
    fn ruleset_without_except_is_plain_allow() {
        let set = RuleSet::allow(vec![Rule::Subpath(PathBuf::from("/nonexistent-a"))]);
        let out = render_ruleset("file-read*", &set).unwrap();
        assert_eq!(
            out,
            r#"(allow file-read* (subpath "/nonexistent-a") (subpath "/System/Volumes/Data/nonexistent-a"))"#
        );
    }

    #[test]
    fn ruleset_with_except_composes_require_not() {
        let set = RuleSet {
            allow: vec![Rule::Subpath(PathBuf::from("/private/wt"))],
            except: vec![Rule::Node(PathBuf::from("/private/wt/.git"))],
        };
        let out = render_ruleset("file-write*", &set).unwrap();
        assert!(out.starts_with("(allow file-write* (require-all (require-any"));
        assert!(out.contains("(require-not (require-any"));
        assert!(out.contains(r#"(regex #"^/private/wt/\.git(/.*)?$")"#));
    }

    #[test]
    fn empty_ruleset_renders_nothing() {
        assert_eq!(render_ruleset("file-read*", &RuleSet::default()), None);
    }
}
