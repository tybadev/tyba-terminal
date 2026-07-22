use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::sandbox::policy::{AgentAccess, Rule, RuleSet};
use crate::sandbox::SandboxSpec;

use super::registry::{entry_for_file, ProfilePath, ServerEntry};

pub struct LspSpecCtx<'a> {
    pub root: &'a Path,
    pub cache_dir: &'a Path,
    pub exe: &'a Path,
    pub env: &'a HashMap<String, String>,
    pub exec_path_dirs: Vec<PathBuf>,
    pub read_allow_extra: Vec<PathBuf>,
    pub extra_reads: Vec<PathBuf>,
    pub data_dir: PathBuf,
    pub data_dir_reads: Vec<PathBuf>,
}

fn resolve_profile_path(path: &ProfilePath, home: &Path, env: &HashMap<String, String>) -> PathBuf {
    match path {
        ProfilePath::Home(rel) => home.join(rel),
        ProfilePath::Abs(abs) => PathBuf::from(abs),
        ProfilePath::Env(var, fallback) => env
            .get(*var)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(fallback)),
    }
}

fn agent_access(entry: &ServerEntry, home: &Path, env: &HashMap<String, String>) -> AgentAccess {
    let mut read_rules: Vec<Rule> = entry
        .reads
        .iter()
        .map(|p| Rule::Subpath(resolve_profile_path(p, home, env)))
        .collect();
    let mut write_rules: Vec<Rule> = Vec::new();
    for path in entry.writes {
        let resolved = resolve_profile_path(path, home, env);
        read_rules.push(Rule::Subpath(resolved.clone()));
        write_rules.push(Rule::Subpath(resolved));
    }
    let mut access = AgentAccess::default();
    if !read_rules.is_empty() {
        access.read.push(RuleSet::allow(read_rules));
    }
    if !write_rules.is_empty() {
        access.write.push(RuleSet::allow(write_rules));
    }
    access
}

pub fn lsp_sandbox_spec(entry: &ServerEntry, ctx: &LspSpecCtx) -> Result<SandboxSpec, String> {
    let home = ctx
        .env
        .get("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .ok_or("HOME indisponível — a jaula do LSP exige a home do usuário")?;

    let inert_git = ctx.cache_dir.join(".no-git");
    let mut agent = agent_access(entry, &home, ctx.env);
    if !ctx.extra_reads.is_empty() {
        agent.read.push(RuleSet::allow(
            ctx.extra_reads
                .iter()
                .map(|p| Rule::Subpath(p.clone()))
                .collect(),
        ));
    }

    Ok(SandboxSpec {
        writable_root: ctx.root.to_path_buf(),
        readable_root: ctx.root.to_path_buf(),
        allow_network: false,
        repo_git_dir: inert_git.clone(),
        worktree_git_dir: inert_git,
        runtime_dir: ctx.cache_dir.join(".rt"),
        hook_socket: ctx.cache_dir.join(".no-hook.sock"),
        tyba_exe: crate::repo::canonicalize_or(ctx.exe),
        tyba_data_dir: ctx.data_dir.clone(),
        home,
        tmpdir: ctx.env.get("TMPDIR").map(PathBuf::from),
        exec_path_dirs: ctx.exec_path_dirs.clone(),
        agent,
        read_allow_extra: ctx.read_allow_extra.clone(),
        data_dir_reads: ctx.data_dir_reads.clone(),
    })
}

pub fn spec_for_file(
    path: &str,
    ctx: &LspSpecCtx,
) -> Result<(&'static ServerEntry, SandboxSpec), String> {
    let (entry, _lang) =
        entry_for_file(path).ok_or("nenhum language server registrado para este arquivo")?;
    let spec = lsp_sandbox_spec(entry, ctx)?;
    Ok((entry, spec))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::registry::entry_by_id;

    fn ctx_env() -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert("HOME".into(), "/Users/nobody".into());
        env.insert("TMPDIR".into(), "/private/var/folders/xx/T".into());
        env
    }

    fn ctx<'a>(
        root: &'a Path,
        cache: &'a Path,
        exe: &'a Path,
        env: &'a HashMap<String, String>,
    ) -> LspSpecCtx<'a> {
        LspSpecCtx {
            root,
            cache_dir: cache,
            exe,
            env,
            exec_path_dirs: vec![PathBuf::from("/opt/homebrew/bin")],
            read_allow_extra: vec![],
            extra_reads: vec![],
            data_dir: PathBuf::from("/Users/nobody/Library/Application Support/dev.tyba.app"),
            data_dir_reads: vec![],
        }
    }

    #[test]
    fn extra_reads_land_in_the_agent_read_allowlist() {
        let env = ctx_env();
        let mut c = ctx(
            Path::new("/private/wt/proj"),
            Path::new("/private/tmp/tyba-lsp-ts"),
            Path::new("/x/tyba"),
            &env,
        );
        c.extra_reads = vec![PathBuf::from("/usr/local/lib/node_modules")];
        let entry = entry_by_id("typescript-language-server").unwrap();
        let spec = lsp_sandbox_spec(entry, &c).unwrap();
        let reads: Vec<PathBuf> = spec
            .agent
            .read
            .iter()
            .flat_map(|set| set.allow.iter())
            .map(|r| r.path().to_path_buf())
            .collect();
        assert!(
            reads.contains(&PathBuf::from("/usr/local/lib/node_modules")),
            "grants derivados da descoberta precisam ser legíveis: {reads:?}"
        );
    }

    #[test]
    fn network_is_always_denied() {
        let env = ctx_env();
        let c = ctx(
            Path::new("/private/wt/proj"),
            Path::new("/private/tmp/tyba-lsp-abc"),
            Path::new("/Apps/Tyba.app/Contents/MacOS/tyba"),
            &env,
        );
        let entry = entry_by_id("rust-analyzer").unwrap();
        let spec = lsp_sandbox_spec(entry, &c).unwrap();
        assert!(!spec.allow_network, "LSP nunca recebe rede");
    }

    #[test]
    fn writable_and_readable_root_are_the_panel_root() {
        let env = ctx_env();
        let root = Path::new("/private/wt/proj");
        let c = ctx(
            root,
            Path::new("/private/tmp/tyba-lsp-abc"),
            Path::new("/x/tyba"),
            &env,
        );
        let entry = entry_by_id("rust-analyzer").unwrap();
        let spec = lsp_sandbox_spec(entry, &c).unwrap();
        assert_eq!(spec.writable_root, root);
        assert_eq!(spec.readable_root, root);
    }

    #[test]
    fn git_dirs_never_point_at_the_real_workspace_git() {
        let env = ctx_env();
        let root = Path::new("/private/wt/proj");
        let cache = Path::new("/private/tmp/tyba-lsp-abc");
        let c = ctx(root, cache, Path::new("/x/tyba"), &env);
        let entry = entry_by_id("rust-analyzer").unwrap();
        let spec = lsp_sandbox_spec(entry, &c).unwrap();
        assert_ne!(spec.repo_git_dir, root.join(".git"));
        assert_ne!(spec.worktree_git_dir, root.join(".git"));
        assert!(spec.repo_git_dir.starts_with(cache));
        assert!(spec.worktree_git_dir.starts_with(cache));
    }

    #[test]
    fn profile_writes_become_readable_and_writable_rules() {
        let env = ctx_env();
        let c = ctx(
            Path::new("/private/wt/proj"),
            Path::new("/private/tmp/tyba-lsp-go"),
            Path::new("/x/tyba"),
            &env,
        );
        let gopls = entry_by_id("gopls").unwrap();
        let spec = lsp_sandbox_spec(gopls, &c).unwrap();
        let write_paths: Vec<PathBuf> = spec
            .agent
            .write
            .iter()
            .flat_map(|set| set.allow.iter())
            .map(|r| r.path().to_path_buf())
            .collect();
        assert!(
            write_paths.contains(&PathBuf::from("/Users/nobody/go/pkg/mod")),
            "o module cache precisa ser gravável: {write_paths:?}"
        );
        assert!(
            !write_paths.contains(&PathBuf::from("/Users/nobody/go")),
            "GOPATH inteiro (com go/bin no PATH) não pode ser gravável: {write_paths:?}"
        );
        let read_paths: Vec<PathBuf> = spec
            .agent
            .read
            .iter()
            .flat_map(|set| set.allow.iter())
            .map(|r| r.path().to_path_buf())
            .collect();
        assert!(
            read_paths.contains(&PathBuf::from("/Users/nobody/go")),
            "GOPATH precisa ser legível: {read_paths:?}"
        );
    }

    #[test]
    fn env_profile_paths_prefer_the_env_var_over_the_fallback() {
        let mut env = ctx_env();
        env.insert("GOMODCACHE".into(), "/opt/modcache".into());
        let c = ctx(
            Path::new("/private/wt/proj"),
            Path::new("/private/tmp/tyba-lsp-go"),
            Path::new("/x/tyba"),
            &env,
        );
        let gopls = entry_by_id("gopls").unwrap();
        let spec = lsp_sandbox_spec(gopls, &c).unwrap();
        let write_paths: Vec<PathBuf> = spec
            .agent
            .write
            .iter()
            .flat_map(|set| set.allow.iter())
            .map(|r| r.path().to_path_buf())
            .collect();
        assert!(write_paths.contains(&PathBuf::from("/opt/modcache")));
        assert!(!write_paths.contains(&PathBuf::from("/Users/nobody/go/pkg/mod")));
    }

    #[test]
    fn missing_home_is_fail_closed() {
        let env = HashMap::new();
        let c = ctx(
            Path::new("/private/wt/proj"),
            Path::new("/private/tmp/tyba-lsp-abc"),
            Path::new("/x/tyba"),
            &env,
        );
        let entry = entry_by_id("rust-analyzer").unwrap();
        assert!(lsp_sandbox_spec(entry, &c).is_err());
    }

    #[test]
    fn a_language_without_a_registry_entry_is_refused() {
        let env = ctx_env();
        let c = ctx(
            Path::new("/private/wt/proj"),
            Path::new("/private/tmp/tyba-lsp-abc"),
            Path::new("/x/tyba"),
            &env,
        );
        assert!(
            spec_for_file("weird.xyz", &c).is_err(),
            "sem perfil no registry, a jaula não é derivada — fail-closed"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn built_policy_denies_workspace_git_write_and_grants_no_general_network() {
        let env = ctx_env();
        let root = Path::new("/private/wt/proj");
        let c = ctx(
            root,
            Path::new("/private/tmp/tyba-lsp-abc"),
            Path::new("/Apps/Tyba.app/Contents/MacOS/tyba"),
            &env,
        );
        let entry = entry_by_id("rust-analyzer").unwrap();
        let spec = lsp_sandbox_spec(entry, &c).unwrap();
        let policy = crate::sandbox::seatbelt::build_policy(&spec);
        for line in policy
            .lines()
            .filter(|l| l.starts_with("(allow file-write*"))
        {
            assert!(
                !line.contains(r#"(subpath "/private/wt/proj")"#) || line.contains("require-not"),
                "a raiz é gravável, mas nunca sem excluir .git: {line}"
            );
        }
        assert!(
            !policy.lines().any(|l| l == "(allow network-outbound)"),
            "a jaula do LSP não pode abrir rede geral"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn managed_subdir_is_readable_while_the_rest_of_the_data_dir_stays_denied() {
        let env = ctx_env();
        let data_dir = PathBuf::from("/Users/nobody/Library/Application Support/dev.tyba.app");
        let managed = data_dir.join("lsp/rust-analyzer/2026-07-20");
        let mut c = ctx(
            Path::new("/private/wt/proj"),
            Path::new("/private/tmp/tyba-lsp-abc"),
            Path::new("/Apps/Tyba.app/Contents/MacOS/tyba"),
            &env,
        );
        c.data_dir = data_dir.clone();
        c.data_dir_reads = vec![managed.clone()];
        let entry = entry_by_id("rust-analyzer").unwrap();
        let spec = lsp_sandbox_spec(entry, &c).unwrap();
        let policy = crate::sandbox::seatbelt::build_policy(&spec);
        let lines: Vec<&str> = policy.lines().collect();
        let deny_idx = lines
            .iter()
            .position(|l| {
                l.starts_with("(deny file-read* file-write*") && l.contains("dev.tyba.app")
            })
            .expect("o data dir do TYBA precisa ter deny de segredo");
        let allow_idx = lines
            .iter()
            .position(|l| l.contains("(allow file-read*") && l.contains("lsp/rust-analyzer"))
            .expect("o subdir gerenciado precisa de read-allow explícito");
        assert!(
            allow_idx > deny_idx,
            "o read-allow do subdir gerenciado tem que vir DEPOIS do deny (última regra vence)"
        );
        let carved = crate::sandbox::policy::render_rule(&crate::sandbox::policy::Rule::Subpath(
            data_dir.join("secrets.db"),
        ));
        assert!(
            !lines[allow_idx + 1..]
                .iter()
                .any(|l| l.contains("(allow file-read*") && l.contains(&carved)),
            "nada além do subdir gerenciado pode furar o deny do data dir"
        );
    }
}
