//! Toolchain do usuário dentro da jaula: o que o PATH do agente pede para ler.
//!
//! O agente recebe o PATH do shell de login do dono (`shell_path::agent_path`,
//! nunca o do repo — ver `repo_config::agent_env`). Muitos toolchains moram em
//! dot-dirs da home (`~/.nvm`, `~/.cargo/bin`, `~/.hermes/node/bin`, `~/.volta`,
//! `~/.pyenv`...), que a jaula nega por default. Este módulo transforma esse
//! PATH no conjunto de dirs que a jaula libera para LEITURA + EXECUÇÃO — nunca
//! escrita — e é a mesma função para Seatbelt e bwrap.
//!
//! Regra, por entrada `E` do PATH (e pela forma canônica `C` de `E`, se for
//! symlink — cada forma é avaliada à parte, e a entrada inteira cai se qualquer
//! forma for recusada, porque o Seatbelt libera a forma canônica de toda regra):
//!
//! 1. `E` relativa ou inexistente → ignorada.
//! 2. Forma igual à home ou ancestral dela (`/`, `/Users`, `~`) → recusada.
//! 3. Forma fora da home → liberada como está (comportamento anterior; dirs de
//!    sistema são de root e ninguém aqui vira raiz de toolchain).
//! 4. Forma dentro da home → precisa passar no [denylist](HOME_DENY) (a forma
//!    não pode estar dentro de um item, nem conter um) e ser um dir do próprio
//!    usuário sem escrita para grupo/outros. Falhou → entrada inteira recusada.
//! 5. Forma dentro da home e num dot-dir (algum componente relativo à home
//!    começa com `.`) cujo último componente é `bin` ou `shims` → também libera
//!    o pai (a "raiz do toolchain": `~/.hermes/node/bin` → `~/.hermes/node`, onde
//!    o `npm` acha `../lib/node_modules`). O pai precisa estar estritamente
//!    abaixo da home, não ser um [contêiner genérico](CONTAINER_DIRS) e passar
//!    pelas mesmas checagens do passo 4; se não passar, só a entrada é liberada.
//!
//! Itens de arquivo sensíveis DENTRO de uma raiz liberada
//! (`~/.cargo/credentials*`) continuam com o deny final de cada plataforma
//! (`secret_denies` no Seatbelt, máscara no bwrap), que vence qualquer allow.
//!
//! O que ainda NÃO funciona na jaula (nada disto abre escrita fora do worktree):
//! instalação global (`npm i -g`, `cargo install`, `pip install --user`,
//! `rustup update`), caches fora de [`CACHE_ENV_VARS`] (cargo registry, go
//! build/mod cache, pnpm store) e binários de `~/.local/bin` que são symlink
//! para outro lugar da home (pipx, por exemplo): só o próprio dir é liberado,
//! nunca `~/.local` inteiro.

use std::path::{Path, PathBuf};

/// Itens da home que nenhuma regra de PATH libera, mesmo que o usuário ponha
/// um deles (ou um dir que os contenha) no PATH.
pub(crate) const HOME_DENY: [&str; 29] = [
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".config/gcloud",
    ".config/gh",
    ".docker",
    ".kube",
    ".netrc",
    ".git-credentials",
    ".npmrc",
    ".pypirc",
    ".claude",
    ".claude.json",
    ".codex",
    ".tyba",
    ".password-store",
    ".local/share/keyrings",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".config/microsoft-edge",
    "Library/Keychains",
    "Library/Application Support",
    "Library/Cookies",
    "Library/Safari",
    "Library/Mail",
    "Library/Messages",
];

/// Dirs genéricos que guardam de tudo (XDG, Library): nunca viram raiz de
/// toolchain. `~/.local/bin` libera só `~/.local/bin`, não `~/.local` inteiro
/// (que tem `share/keyrings`, `share/claude`...).
pub(crate) const CONTAINER_DIRS: [&str; 6] = [
    ".local",
    ".local/share",
    ".local/state",
    ".config",
    ".cache",
    "Library",
];

/// Último componente que marca um dir de executáveis dentro de um prefixo.
const LIFT_NAMES: [&str; 2] = ["bin", "shims"];

/// O que a regra precisa saber do disco — injetado para testar dono/permissão
/// sem precisar de root.
pub(crate) trait FsView {
    fn canonicalize(&self, path: &Path) -> Option<PathBuf>;
    /// `(uid do dono, mode)` de um diretório; `None` se não existe ou não é dir.
    fn dir_owner_mode(&self, path: &Path) -> Option<(u32, u32)>;
    fn uid(&self) -> u32;
}

pub(crate) struct RealFs;

impl FsView for RealFs {
    fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }

    #[cfg(unix)]
    fn dir_owner_mode(&self, path: &Path) -> Option<(u32, u32)> {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).ok()?;
        meta.is_dir().then(|| (meta.uid(), meta.mode()))
    }

    #[cfg(not(unix))]
    fn dir_owner_mode(&self, _path: &Path) -> Option<(u32, u32)> {
        None
    }

    #[cfg(unix)]
    fn uid(&self) -> u32 {
        // SAFETY: geteuid não tem pré-condição e não falha.
        unsafe { libc::geteuid() }
    }

    #[cfg(not(unix))]
    fn uid(&self) -> u32 {
        u32::MAX
    }
}

/// Dirs que a jaula libera para leitura+execução a partir do PATH do agente.
/// `extra_deny` soma paths absolutos ao denylist (o data dir do Tyba).
pub(crate) fn path_read_grants(
    path_dirs: &[PathBuf],
    home: &Path,
    extra_deny: &[PathBuf],
) -> Vec<PathBuf> {
    path_read_grants_with(&RealFs, path_dirs, home, extra_deny)
}

pub(crate) fn path_read_grants_with(
    fs: &dyn FsView,
    path_dirs: &[PathBuf],
    home: &Path,
    extra_deny: &[PathBuf],
) -> Vec<PathBuf> {
    let mut homes = vec![home.to_path_buf()];
    if let Some(canon) = fs.canonicalize(home) {
        if !homes.contains(&canon) {
            homes.push(canon);
        }
    }
    let mut deny: Vec<PathBuf> = homes
        .iter()
        .flat_map(|h| HOME_DENY.iter().map(move |d| h.join(d)))
        .collect();
    deny.extend(extra_deny.iter().cloned());

    let ctx = Ctx {
        fs,
        homes: &homes,
        deny: &deny,
    };
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in path_dirs {
        for grant in ctx.entry_grants(entry).unwrap_or_default() {
            if !out.contains(&grant) {
                out.push(grant);
            }
        }
    }
    out
}

struct Ctx<'a> {
    fs: &'a dyn FsView,
    homes: &'a [PathBuf],
    deny: &'a [PathBuf],
}

impl Ctx<'_> {
    /// `None` = entrada recusada inteira.
    fn entry_grants(&self, entry: &Path) -> Option<Vec<PathBuf>> {
        if !entry.is_absolute() {
            return None;
        }
        let canon = self.fs.canonicalize(entry)?;
        let mut forms = vec![entry.to_path_buf()];
        if canon != entry {
            forms.push(canon);
        }
        let mut grants = Vec::new();
        for form in &forms {
            grants.extend(self.form_grants(form)?);
        }
        Some(grants)
    }

    fn form_grants(&self, form: &Path) -> Option<Vec<PathBuf>> {
        if self.homes.iter().any(|h| h.starts_with(form)) {
            return None;
        }
        let Some(home) = self.homes.iter().find(|h| form.starts_with(h)) else {
            return Some(vec![form.to_path_buf()]);
        };
        if !self.safe_home_dir(form) {
            return None;
        }
        let mut grants = vec![form.to_path_buf()];
        if let Some(root) = toolchain_root(form, home) {
            if self.safe_home_dir(&root) {
                grants.push(root);
            }
        }
        Some(grants)
    }

    fn safe_home_dir(&self, path: &Path) -> bool {
        let denied = self
            .deny
            .iter()
            .any(|d| path.starts_with(d) || d.starts_with(path));
        if denied {
            return false;
        }
        match self.fs.dir_owner_mode(path) {
            Some((owner, mode)) => owner == self.fs.uid() && mode & 0o022 == 0,
            None => false,
        }
    }
}

/// Raiz de toolchain de `dir` (já sabido dentro de `home`), se a regra do passo
/// 5 se aplica: `dir` num dot-dir, terminando em `bin`/`shims`, com pai
/// estritamente abaixo da home e que não seja contêiner genérico.
pub(crate) fn toolchain_root(dir: &Path, home: &Path) -> Option<PathBuf> {
    let rel = dir.strip_prefix(home).ok()?;
    let hidden = rel
        .components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
    if !hidden {
        return None;
    }
    let name = dir.file_name()?.to_str()?;
    if !LIFT_NAMES.contains(&name) {
        return None;
    }
    let parent = dir.parent()?;
    let parent_rel = parent.strip_prefix(home).ok()?;
    if parent_rel.as_os_str().is_empty()
        || CONTAINER_DIRS.iter().any(|c| parent_rel == Path::new(c))
    {
        return None;
    }
    Some(parent.to_path_buf())
}

/// Variáveis de cache que apontam os toolchains para um dir gravável e
/// exclusivo da sessão, já que a jaula não abre `~/.npm`, `~/.cache/pip` etc.
/// para escrita. Só variáveis de CACHE: `CARGO_HOME`/`RUSTUP_HOME` moveriam
/// binários e toolchains junto, e ficam de fora.
pub(crate) const CACHE_ENV_VARS: [(&str, &str); 4] = [
    ("npm_config_cache", "npm"),
    ("YARN_CACHE_FOLDER", "yarn"),
    ("PIP_CACHE_DIR", "pip"),
    ("BUN_INSTALL_CACHE_DIR", "bun"),
];

/// `(variável, dir)` para a sessão cuja jaula tem `runtime_dir` e `TMPDIR`
/// `tmpdir`. O dir mora no TMPDIR do agente (gravável na jaula: no Seatbelt
/// fora do prefixo `tyba-` reservado; no bwrap é um tmpfs privado) e o nome
/// carrega o id do runtime da sessão, para duas sessões não dividirem cache.
pub(crate) fn cache_env(tmpdir: Option<&Path>, runtime_dir: &Path) -> Vec<(&'static str, PathBuf)> {
    let tmp = tmpdir
        .filter(|t| t.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let tag = runtime_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tag = tag.strip_prefix("tyba-").unwrap_or(&tag);
    let base = tmp.join(format!("toolchain-cache-{tag}"));
    CACHE_ENV_VARS
        .iter()
        .map(|(var, sub)| (*var, base.join(sub)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Disco de verdade, com dono/mode sobrescrevível por path.
    struct Fs {
        overrides: HashMap<PathBuf, (u32, u32)>,
    }

    impl Fs {
        fn real() -> Self {
            Fs {
                overrides: HashMap::new(),
            }
        }
    }

    impl FsView for Fs {
        fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
            RealFs.canonicalize(path)
        }
        fn dir_owner_mode(&self, path: &Path) -> Option<(u32, u32)> {
            let canon = RealFs.canonicalize(path)?;
            self.overrides
                .get(&canon)
                .copied()
                .or_else(|| RealFs.dir_owner_mode(path))
        }
        fn uid(&self) -> u32 {
            RealFs.uid()
        }
    }

    fn home() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap().join("home");
        std::fs::create_dir(&home).unwrap();
        (tmp, home)
    }

    fn mkdir(p: &Path) -> PathBuf {
        std::fs::create_dir_all(p).unwrap();
        p.to_path_buf()
    }

    fn grants(fs: &Fs, home: &Path, path: &[PathBuf]) -> Vec<PathBuf> {
        path_read_grants_with(fs, path, home, &[])
    }

    #[test]
    fn dot_dir_bin_lifts_to_the_install_prefix() {
        let (_t, home) = home();
        let bin = mkdir(&home.join(".hermes/node/bin"));
        mkdir(&home.join(".hermes/node/lib/node_modules"));
        let g = grants(&Fs::real(), &home, std::slice::from_ref(&bin));
        assert_eq!(g, vec![bin.clone(), home.join(".hermes/node")]);
        assert!(
            !g.contains(&home.join(".hermes")),
            "só o prefixo do node, não o dot-dir inteiro"
        );
    }

    #[test]
    fn shims_lift_like_bin() {
        let (_t, home) = home();
        let shims = mkdir(&home.join(".pyenv/shims"));
        let g = grants(&Fs::real(), &home, std::slice::from_ref(&shims));
        assert_eq!(g, vec![shims, home.join(".pyenv")]);
    }

    #[test]
    fn non_bin_dot_entry_is_granted_alone() {
        let (_t, home) = home();
        let dir = mkdir(&home.join(".hermes/tools"));
        assert_eq!(
            grants(&Fs::real(), &home, std::slice::from_ref(&dir)),
            vec![dir]
        );
    }

    #[test]
    fn non_hidden_home_entry_is_never_lifted() {
        let (_t, home) = home();
        let bin = mkdir(&home.join("go/bin"));
        assert_eq!(
            grants(&Fs::real(), &home, std::slice::from_ref(&bin)),
            vec![bin]
        );
    }

    #[test]
    fn generic_containers_are_never_the_root() {
        let (_t, home) = home();
        let local_bin = mkdir(&home.join(".local/bin"));
        let share_bin = mkdir(&home.join(".local/share/bin"));
        let config_bin = mkdir(&home.join(".config/bin"));
        let top_bin = mkdir(&home.join(".bin"));
        let g = grants(
            &Fs::real(),
            &home,
            &[
                local_bin.clone(),
                share_bin.clone(),
                config_bin.clone(),
                top_bin.clone(),
            ],
        );
        assert_eq!(g, vec![local_bin, share_bin, config_bin, top_bin]);
    }

    #[test]
    fn toolchain_root_rule_is_exact() {
        let h = Path::new("/h");
        let root = |d: &str| toolchain_root(Path::new(d), h);
        assert_eq!(root("/h/.cargo/bin"), Some(PathBuf::from("/h/.cargo")));
        assert_eq!(
            root("/h/.nvm/versions/node/v20.1.0/bin"),
            Some(PathBuf::from("/h/.nvm/versions/node/v20.1.0"))
        );
        assert_eq!(
            root("/h/.local/share/mise/installs/node/20/bin"),
            Some(PathBuf::from("/h/.local/share/mise/installs/node/20"))
        );
        assert_eq!(
            root("/h/work/.venv/bin"),
            Some(PathBuf::from("/h/work/.venv"))
        );
        assert_eq!(root("/h/.local/bin"), None);
        assert_eq!(root("/h/bin"), None, "pai seria a home");
        assert_eq!(root("/h/go/bin"), None, "não é dot-dir");
        assert_eq!(root("/h/.cargo/binx"), None);
        assert_eq!(root("/elsewhere/.x/bin"), None, "fora da home");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_entry_grants_the_canonical_target_and_its_prefix() {
        let (_t, home) = home();
        let real_bin = mkdir(&home.join(".toolchains/node-20/bin"));
        mkdir(&home.join(".hermes"));
        let link = home.join(".hermes/node");
        std::os::unix::fs::symlink(home.join(".toolchains/node-20"), &link).unwrap();
        let entry = link.join("bin");
        let g = grants(&Fs::real(), &home, std::slice::from_ref(&entry));
        assert!(g.contains(&entry));
        assert!(g.contains(&link));
        assert!(g.contains(&real_bin));
        assert!(g.contains(&home.join(".toolchains/node-20")));
        assert!(!g.contains(&home.join(".toolchains")));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_into_a_denied_dir_drops_the_whole_entry() {
        let (_t, home) = home();
        let secret_bin = mkdir(&home.join(".ssh/bin"));
        mkdir(&home.join(".tool"));
        let link = home.join(".tool/bin");
        std::os::unix::fs::symlink(&secret_bin, &link).unwrap();
        assert!(
            grants(&Fs::real(), &home, &[link]).is_empty(),
            "o Seatbelt libera a forma canônica de toda regra: liberar o link \
             liberaria ~/.ssh"
        );
    }

    #[test]
    fn denylist_wins_over_path() {
        let (_t, home) = home();
        let mut path = Vec::new();
        for d in [
            ".ssh",
            ".ssh/bin",
            ".gnupg/bin",
            ".aws/bin",
            ".config/gcloud/bin",
            ".docker/bin",
            ".kube/bin",
            ".claude/local/bin",
            ".codex/bin",
            "Library/Keychains",
            "Library/Application Support/Google/Chrome/bin",
        ] {
            path.push(mkdir(&home.join(d)));
        }
        assert!(grants(&Fs::real(), &home, &path).is_empty());
    }

    #[test]
    fn an_entry_that_contains_a_denied_dir_is_refused() {
        let (_t, home) = home();
        let local = mkdir(&home.join(".local"));
        mkdir(&home.join(".local/share/keyrings"));
        assert!(grants(&Fs::real(), &home, &[local]).is_empty());
    }

    #[test]
    fn extra_deny_blocks_the_lift_but_keeps_an_unrelated_entry() {
        let (_t, home) = home();
        let bin = mkdir(&home.join(".tool/bin"));
        let secret = mkdir(&home.join(".tool/secret"));
        let g = path_read_grants_with(&Fs::real(), std::slice::from_ref(&bin), &home, &[secret]);
        assert_eq!(g, vec![bin], "a raiz conteria o dir negado; só a entrada");
    }

    #[test]
    fn home_and_its_ancestors_are_rejected() {
        let (_t, home) = home();
        let parent = home.parent().unwrap().to_path_buf();
        let g = grants(
            &Fs::real(),
            &home,
            &[
                home.clone(),
                parent,
                PathBuf::from("/"),
                PathBuf::from("relative/bin"),
            ],
        );
        assert!(g.is_empty(), "{g:?}");
    }

    #[test]
    fn outside_home_entries_keep_the_old_behavior() {
        let (_t, home) = home();
        let canon = std::fs::canonicalize("/bin").unwrap();
        let g = grants(&Fs::real(), &home, &[PathBuf::from("/bin")]);
        assert!(
            g.contains(&PathBuf::from("/bin")) && g.contains(&canon),
            "{g:?}"
        );
        assert!(g.iter().all(|p| p == Path::new("/bin") || *p == canon));
    }

    #[test]
    fn missing_entries_are_skipped() {
        let (_t, home) = home();
        assert!(grants(&Fs::real(), &home, &[home.join(".nope/bin")]).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn group_or_world_writable_dirs_are_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let (_t, home) = home();
        let bin = mkdir(&home.join(".shared/bin"));
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o775)).unwrap();
        assert!(grants(&Fs::real(), &home, std::slice::from_ref(&bin)).is_empty());
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o757)).unwrap();
        assert!(grants(&Fs::real(), &home, std::slice::from_ref(&bin)).is_empty());

        let bin2 = mkdir(&home.join(".tool/bin"));
        std::fs::set_permissions(home.join(".tool"), std::fs::Permissions::from_mode(0o777))
            .unwrap();
        assert_eq!(
            grants(&Fs::real(), &home, std::slice::from_ref(&bin2)),
            vec![bin2],
            "raiz gravável por outros não é liberada; a entrada segura sim"
        );
    }

    #[test]
    fn dirs_not_owned_by_the_user_are_skipped() {
        let (_t, home) = home();
        let bin = mkdir(&home.join(".foreign/bin"));
        let mut fs = Fs::real();
        fs.overrides.insert(
            bin.canonicalize().unwrap(),
            (fs.uid().wrapping_add(1), 0o40755),
        );
        assert!(grants(&fs, &home, &[bin]).is_empty());

        let bin = mkdir(&home.join(".mine/bin"));
        let mut fs = Fs::real();
        fs.overrides.insert(
            home.join(".mine").canonicalize().unwrap(),
            (fs.uid().wrapping_add(1), 0o40755),
        );
        assert_eq!(grants(&fs, &home, std::slice::from_ref(&bin)), vec![bin]);
    }

    #[test]
    fn cache_env_is_per_session_under_tmpdir_and_avoids_the_tyba_prefix() {
        let env = cache_env(
            Some(Path::new("/private/var/folders/xx/T")),
            Path::new("/private/var/folders/xx/T/tyba-0123456789ab"),
        );
        let vars: Vec<&str> = env.iter().map(|(v, _)| *v).collect();
        assert_eq!(
            vars,
            [
                "npm_config_cache",
                "YARN_CACHE_FOLDER",
                "PIP_CACHE_DIR",
                "BUN_INSTALL_CACHE_DIR"
            ]
        );
        for (_, dir) in &env {
            assert!(dir.starts_with("/private/var/folders/xx/T/toolchain-cache-0123456789ab"));
        }
        assert!(
            !vars.contains(&"CARGO_HOME"),
            "CARGO_HOME moveria os binários"
        );
    }

    #[test]
    fn cache_env_falls_back_to_tmp() {
        let env = cache_env(None, Path::new("/tmp/tyba-abc"));
        assert_eq!(env[0].1, PathBuf::from("/tmp/toolchain-cache-abc/npm"));
        let env = cache_env(Some(Path::new("relative")), Path::new("/tmp/tyba-abc"));
        assert_eq!(env[0].1, PathBuf::from("/tmp/toolchain-cache-abc/npm"));
    }
}
