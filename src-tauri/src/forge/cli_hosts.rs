use std::fs;
use std::path::PathBuf;

use super::ForgeKind;

pub fn kind_for_host(host: &str) -> Option<ForgeKind> {
    let host = host.to_ascii_lowercase();
    if gh_hosts().iter().any(|h| h == &host) {
        return Some(ForgeKind::GitHub);
    }
    if glab_hosts().iter().any(|h| h == &host) {
        return Some(ForgeKind::GitLab);
    }
    None
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn gh_hosts() -> Vec<String> {
    let Some(home) = home() else {
        return Vec::new();
    };
    let content = fs::read_to_string(home.join(".config/gh/hosts.yml")).unwrap_or_default();
    top_level_keys(&content)
}

fn glab_hosts() -> Vec<String> {
    let Some(home) = home() else {
        return Vec::new();
    };
    let candidates = [
        home.join("Library/Application Support/glab-cli/config.yml"),
        home.join(".config/glab-cli/config.yml"),
    ];
    for path in candidates {
        if let Ok(content) = fs::read_to_string(&path) {
            return hosts_section_keys(&content);
        }
    }
    Vec::new()
}

fn key_of(line: &str) -> Option<String> {
    let key = line.trim().strip_suffix(':')?;
    if key.is_empty() || key.contains(char::is_whitespace) || key.contains('"') {
        return None;
    }
    Some(key.to_ascii_lowercase())
}

fn top_level_keys(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| !line.starts_with(char::is_whitespace))
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(key_of)
        .collect()
}

fn hosts_section_keys(content: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut in_hosts = false;
    let mut host_indent: Option<usize> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            in_hosts = trimmed == "hosts:";
            host_indent = None;
            continue;
        }
        if !in_hosts {
            continue;
        }
        let level = *host_indent.get_or_insert(indent);
        if indent == level {
            if let Some(key) = key_of(line) {
                keys.push(key);
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_gh_hosts_from_top_level_keys() {
        let content = "\
# comment
github.com:
    user: alice
    git_protocol: https
ghe.acme.com:
    user: bob
";
        assert_eq!(top_level_keys(content), vec!["github.com", "ghe.acme.com"]);
    }

    #[test]
    fn extracts_glab_hosts_from_the_hosts_section_only() {
        let content = "\
git_protocol: ssh
host: gitlab.com
hosts:
    gitlab.com:
        token: SECRET
        user: alice
    git.4all.com:
        token: SECRET
check_update: true
";
        assert_eq!(
            hosts_section_keys(content),
            vec!["gitlab.com", "git.4all.com"]
        );
    }

    #[test]
    fn ignores_value_lines_and_files_without_a_hosts_section() {
        let content = "git_protocol: ssh\nhost: gitlab.com\n";
        assert!(hosts_section_keys(content).is_empty());
        assert_eq!(key_of("    git_protocol: ssh"), None);
        assert_eq!(key_of("    token:"), Some("token".into()));
    }

    #[test]
    fn keys_are_lowercased_for_comparison() {
        assert_eq!(top_level_keys("GHE.Acme.Com:\n"), vec!["ghe.acme.com"]);
    }
}
