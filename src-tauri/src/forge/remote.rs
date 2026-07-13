use super::ForgeKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

impl Remote {
    pub fn kind(&self) -> Option<ForgeKind> {
        let host = self.host.to_ascii_lowercase();
        if host == "github.com" || host.starts_with("github.") || host.contains("github") {
            return Some(ForgeKind::GitHub);
        }
        if host == "gitlab.com" || host.starts_with("gitlab.") || host.contains("gitlab") {
            return Some(ForgeKind::GitLab);
        }
        if self.owner.contains('/') {
            return Some(ForgeKind::GitLab);
        }
        None
    }

    pub fn project(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    pub fn base_url(&self) -> String {
        format!("https://{}/{}/{}", self.host, self.owner, self.repo)
    }
}

pub fn parse(url: &str) -> Option<Remote> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    let (host, path) = if let Some(rest) = strip_scheme(url) {
        split_host_path(rest)?
    } else if let Some((authority, path)) = url.split_once(':') {
        let host = authority.rsplit('@').next()?;
        (host.to_string(), path.to_string())
    } else {
        return None;
    };

    if host.is_empty() {
        return None;
    }

    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let path = path.trim_matches('/');
    let (owner, repo) = path.rsplit_once('/')?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some(Remote {
        host: strip_port(&host),
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

fn strip_scheme(url: &str) -> Option<&str> {
    for scheme in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            return Some(rest);
        }
    }
    None
}

fn split_host_path(rest: &str) -> Option<(String, String)> {
    let (authority, path) = rest.split_once('/')?;
    let host = authority.rsplit('@').next()?;
    Some((host.to_string(), path.to_string()))
}

fn strip_port(host: &str) -> String {
    match host.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => {
            h.to_string()
        }
        _ => host.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_remote() {
        let r = parse("https://github.com/tybadev/tyba-terminal.git").unwrap();
        assert_eq!(r.host, "github.com");
        assert_eq!(r.owner, "tybadev");
        assert_eq!(r.repo, "tyba-terminal");
        assert_eq!(r.kind(), Some(ForgeKind::GitHub));
    }

    #[test]
    fn parses_scp_like_ssh_remote() {
        let r = parse("git@github.com:owner/repo.git").unwrap();
        assert_eq!(r.host, "github.com");
        assert_eq!(r.project(), "owner/repo");
    }

    #[test]
    fn parses_ssh_scheme_remote() {
        let r = parse("ssh://git@gitlab.com/group/repo.git").unwrap();
        assert_eq!(r.host, "gitlab.com");
        assert_eq!(r.owner, "group");
        assert_eq!(r.kind(), Some(ForgeKind::GitLab));
    }

    #[test]
    fn keeps_gitlab_subgroups_in_the_owner() {
        let r = parse("git@gitlab.com:group/subgroup/repo.git").unwrap();
        assert_eq!(r.owner, "group/subgroup");
        assert_eq!(r.repo, "repo");
        assert_eq!(
            r.base_url(),
            "https://gitlab.com/group/subgroup/repo".to_string()
        );
    }

    #[test]
    fn drops_userinfo_and_port() {
        let r = parse("https://user@gitlab.example.com:8443/team/app.git").unwrap();
        assert_eq!(r.host, "gitlab.example.com");
        assert_eq!(r.kind(), Some(ForgeKind::GitLab));
    }

    #[test]
    fn parses_remote_without_git_suffix() {
        let r = parse("https://github.com/owner/repo").unwrap();
        assert_eq!(r.repo, "repo");
    }

    #[test]
    fn subgroups_mark_a_custom_host_as_gitlab() {
        let r = parse("git@git.acme.com:group/sub/infra/app.git").unwrap();
        assert_eq!(r.host, "git.acme.com");
        assert_eq!(r.owner, "group/sub/infra");
        assert_eq!(r.kind(), Some(ForgeKind::GitLab));
    }

    #[test]
    fn ignores_unknown_hosts_and_garbage() {
        assert_eq!(parse("https://bitbucket.org/o/r.git").unwrap().kind(), None);
        assert!(parse("").is_none());
        assert!(parse("not-a-url").is_none());
        assert!(parse("https://github.com/onlyowner").is_none());
    }
}
