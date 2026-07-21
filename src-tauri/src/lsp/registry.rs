use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Native,
    Node,
    Php,
    Jvm,
    Beam,
}

impl Runtime {
    pub fn interpreter(&self) -> Option<&'static str> {
        match self {
            Runtime::Native => None,
            Runtime::Node => Some("node"),
            Runtime::Php => Some("php"),
            Runtime::Jvm => Some("java"),
            Runtime::Beam => Some("elixir"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Rustup,
    Go,
    Cargo,
    Composer,
    Mix,
    Brew,
    Bun,
    Pipx,
    Scoop,
    Npm,
    Manual,
}

impl Manager {
    pub fn binary(&self) -> Option<&'static str> {
        match self {
            Manager::Rustup => Some("rustup"),
            Manager::Go => Some("go"),
            Manager::Cargo => Some("cargo"),
            Manager::Composer => Some("composer"),
            Manager::Mix => Some("mix"),
            Manager::Brew => Some("brew"),
            Manager::Bun => Some("bun"),
            Manager::Pipx => Some("pipx"),
            Manager::Scoop => Some("scoop"),
            Manager::Npm => Some("npm"),
            Manager::Manual => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Manager::Rustup => "rustup",
            Manager::Go => "go",
            Manager::Cargo => "cargo",
            Manager::Composer => "composer",
            Manager::Mix => "mix",
            Manager::Brew => "brew",
            Manager::Bun => "bun",
            Manager::Pipx => "pipx",
            Manager::Scoop => "scoop",
            Manager::Npm => "npm",
            Manager::Manual => "manual",
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Manager::Rustup | Manager::Go => 0,
            Manager::Cargo | Manager::Composer | Manager::Mix => 1,
            Manager::Brew => 2,
            Manager::Bun => 3,
            Manager::Pipx => 4,
            Manager::Scoop => 5,
            Manager::Npm => 6,
            Manager::Manual => 10,
        }
    }

    pub fn available(&self, path_dirs: &[PathBuf]) -> bool {
        match self.binary() {
            None => true,
            Some(bin) => path_dirs.iter().any(|dir| {
                candidate_names(bin)
                    .iter()
                    .any(|name| is_executable(&dir.join(name)))
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Install {
    pub manager: Manager,
    pub command: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub enum BinDir {
    HomeCargoBin,
    HomeGoBin,
    HomeLocalBin,
    NodeModulesBin,
}

#[derive(Debug, Clone, Copy)]
pub enum ProfilePath {
    Home(&'static str),
    Abs(&'static str),
    Env(&'static str, &'static str),
}

impl ProfilePath {
    pub fn env_var(&self) -> Option<&'static str> {
        match self {
            ProfilePath::Env(var, _) => Some(var),
            _ => None,
        }
    }
}

pub struct ServerEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub runtime: Runtime,
    pub binaries: &'static [&'static str],
    pub args: &'static [&'static str],
    pub extensions: &'static [(&'static str, &'static str)],
    pub filenames: &'static [(&'static str, &'static str)],
    pub dockerfile_prefix: bool,
    pub extra_bin_dirs: &'static [BinDir],
    pub install: &'static [Install],
    pub init_options: Option<&'static str>,
    pub reads: &'static [ProfilePath],
    pub writes: &'static [ProfilePath],
    pub experimental: bool,
    pub default_enabled: bool,
}

impl ServerEntry {
    pub fn forwarded_env(&self) -> Vec<&'static str> {
        let mut vars: Vec<&'static str> = Vec::new();
        for path in self.reads.iter().chain(self.writes.iter()) {
            if let Some(var) = path.env_var() {
                if !vars.contains(&var) {
                    vars.push(var);
                }
            }
        }
        vars
    }
}

macro_rules! installs {
    ($($mgr:ident : $cmd:literal),+ $(,)?) => {
        &[$(Install { manager: Manager::$mgr, command: $cmd }),+]
    };
}

pub static REGISTRY: &[ServerEntry] = &[
    ServerEntry {
        id: "rust-analyzer",
        label: "rust-analyzer",
        runtime: Runtime::Native,
        binaries: &["rust-analyzer"],
        args: &[],
        extensions: &[("rs", "rust")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeCargoBin],
        install: installs!(
            Rustup: "rustup component add rust-analyzer",
            Brew: "brew install rust-analyzer",
        ),
        init_options: None,
        reads: &[],
        writes: &[],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "typescript-language-server",
        label: "typescript-language-server",
        runtime: Runtime::Node,
        binaries: &["typescript-language-server"],
        args: &["--stdio"],
        extensions: &[
            ("ts", "typescript"),
            ("mts", "typescript"),
            ("cts", "typescript"),
            ("tsx", "typescriptreact"),
            ("js", "javascript"),
            ("mjs", "javascript"),
            ("cjs", "javascript"),
            ("jsx", "javascriptreact"),
        ],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: installs!(
            Bun: "bun add -g typescript-language-server typescript",
            Npm: "npm install -g typescript-language-server typescript",
        ),
        init_options: None,
        reads: &[],
        writes: &[],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "pyright",
        label: "Pyright",
        runtime: Runtime::Node,
        binaries: &["pyright-langserver"],
        args: &["--stdio"],
        extensions: &[("py", "python"), ("pyi", "python")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: installs!(
            Brew: "brew install pyright",
            Pipx: "pipx install pyright",
            Bun: "bun add -g pyright",
            Npm: "npm install -g pyright",
        ),
        init_options: None,
        reads: &[],
        writes: &[],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "gopls",
        label: "gopls",
        runtime: Runtime::Native,
        binaries: &["gopls"],
        args: &[],
        extensions: &[("go", "go")],
        filenames: &[("go.mod", "go.mod"), ("go.sum", "go.sum")],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeGoBin],
        install: installs!(
            Go: "go install golang.org/x/tools/gopls@latest",
            Brew: "brew install gopls",
        ),
        init_options: None,
        reads: &[ProfilePath::Env("GOPATH", "go")],
        writes: &[
            ProfilePath::Env("GOMODCACHE", "go/pkg/mod"),
            ProfilePath::Env("GOCACHE", "Library/Caches/go-build"),
            ProfilePath::Home(".cache/go-build"),
        ],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "bash-language-server",
        label: "bash-language-server",
        runtime: Runtime::Node,
        binaries: &["bash-language-server"],
        args: &["start"],
        extensions: &[
            ("sh", "shellscript"),
            ("bash", "shellscript"),
            ("zsh", "shellscript"),
        ],
        filenames: &[
            (".bashrc", "shellscript"),
            (".zshrc", "shellscript"),
            (".bash_profile", "shellscript"),
        ],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: installs!(
            Brew: "brew install bash-language-server",
            Bun: "bun add -g bash-language-server",
            Npm: "npm install -g bash-language-server",
        ),
        init_options: None,
        reads: &[],
        writes: &[],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "yaml-language-server",
        label: "yaml-language-server",
        runtime: Runtime::Node,
        binaries: &["yaml-language-server"],
        args: &["--stdio"],
        extensions: &[("yaml", "yaml"), ("yml", "yaml")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: installs!(
            Bun: "bun add -g yaml-language-server",
            Npm: "npm install -g yaml-language-server",
        ),
        init_options: Some(
            r#"{"yaml":{"validate":true,"hover":true,"completion":true,"schemaStore":{"enable":false},"schemas":{"https://raw.githubusercontent.com/compose-spec/compose-spec/master/schema/compose-spec.json":["docker-compose*.yml","docker-compose*.yaml","compose*.yml","compose*.yaml"],"https://json.schemastore.org/github-workflow.json":[".github/workflows/*.yml",".github/workflows/*.yaml"],"kubernetes":["*.k8s.yaml","*.k8s.yml"]}}}"#,
        ),
        reads: &[],
        writes: &[],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "taplo",
        label: "Taplo (TOML)",
        runtime: Runtime::Native,
        binaries: &["taplo"],
        args: &["lsp", "stdio"],
        extensions: &[("toml", "toml")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeCargoBin],
        install: installs!(
            Brew: "brew install taplo",
            Cargo: "cargo install taplo-cli --locked",
        ),
        init_options: None,
        reads: &[],
        writes: &[],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "dockerfile-language-server",
        label: "dockerfile-language-server",
        runtime: Runtime::Node,
        binaries: &["docker-langserver"],
        args: &["--stdio"],
        extensions: &[("dockerfile", "dockerfile")],
        filenames: &[
            ("dockerfile", "dockerfile"),
            ("containerfile", "dockerfile"),
        ],
        dockerfile_prefix: true,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: installs!(
            Bun: "bun add -g dockerfile-language-server-nodejs",
            Npm: "npm install -g dockerfile-language-server-nodejs",
        ),
        init_options: None,
        reads: &[],
        writes: &[],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "marksman",
        label: "Marksman (Markdown)",
        runtime: Runtime::Native,
        binaries: &["marksman"],
        args: &["server"],
        extensions: &[("md", "markdown"), ("markdown", "markdown")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: installs!(
            Brew: "brew install marksman",
            Scoop: "scoop install marksman",
            Manual: "Baixe marksman em github.com/artempyanykh/marksman/releases e ponha no PATH",
        ),
        init_options: None,
        reads: &[],
        writes: &[ProfilePath::Home(".cache/marksman")],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "vscode-json-language-server",
        label: "vscode-json-language-server",
        runtime: Runtime::Node,
        binaries: &["vscode-json-language-server"],
        args: &["--stdio"],
        extensions: &[("json", "json"), ("jsonc", "jsonc")],
        filenames: &[("tsconfig.json", "jsonc"), (".babelrc", "jsonc")],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: installs!(
            Bun: "bun add -g vscode-langservers-extracted",
            Npm: "npm install -g vscode-langservers-extracted",
        ),
        init_options: None,
        reads: &[],
        writes: &[],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "terraform-ls",
        label: "terraform-ls",
        runtime: Runtime::Native,
        binaries: &["terraform-ls"],
        args: &["serve"],
        extensions: &[("tf", "terraform"), ("tfvars", "terraform")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: installs!(
            Brew: "brew install hashicorp/tap/terraform-ls",
            Scoop: "scoop install terraform-ls",
        ),
        init_options: None,
        reads: &[ProfilePath::Home(".terraform.d")],
        writes: &[],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "jdtls",
        label: "Eclipse JDT (Java)",
        runtime: Runtime::Jvm,
        binaries: &["jdtls"],
        args: &[],
        extensions: &[("java", "java")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: installs!(
            Brew: "brew install jdtls",
            Manual: "Baixe o Eclipse JDT LS e ponha `jdtls` no PATH",
        ),
        init_options: None,
        reads: &[ProfilePath::Home(".config/jdtls")],
        writes: &[
            ProfilePath::Home(".cache/jdtls"),
            ProfilePath::Home(".local/share/jdtls"),
        ],
        experimental: true,
        default_enabled: false,
    },
    ServerEntry {
        id: "clojure-lsp",
        label: "clojure-lsp",
        runtime: Runtime::Native,
        binaries: &["clojure-lsp"],
        args: &[],
        extensions: &[
            ("clj", "clojure"),
            ("cljs", "clojurescript"),
            ("cljc", "clojure"),
            ("edn", "clojure"),
        ],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: installs!(
            Brew: "brew install clojure-lsp/brew/clojure-lsp-native",
            Scoop: "scoop install clojure-lsp",
            Manual: "Baixe clojure-lsp em github.com/clojure-lsp/clojure-lsp/releases",
        ),
        init_options: None,
        reads: &[ProfilePath::Home(".m2"), ProfilePath::Home(".gitlibs")],
        writes: &[
            ProfilePath::Home(".cache/clojure-lsp"),
            ProfilePath::Home(".m2"),
        ],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "elixir-ls",
        label: "ElixirLS",
        runtime: Runtime::Beam,
        binaries: &["elixir-ls", "language_server.sh"],
        args: &[],
        extensions: &[("ex", "elixir"), ("exs", "elixir")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: installs!(
            Brew: "brew install elixir-ls",
            Manual: "Baixe ElixirLS em github.com/elixir-lsp/elixir-ls/releases",
        ),
        init_options: None,
        reads: &[ProfilePath::Home(".mix"), ProfilePath::Home(".hex")],
        writes: &[ProfilePath::Home(".mix"), ProfilePath::Home(".hex")],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "phpactor",
        label: "Phpactor (PHP)",
        runtime: Runtime::Php,
        binaries: &["phpactor"],
        args: &["language-server"],
        extensions: &[("php", "php")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: installs!(
            Composer: "composer global require phpactor/phpactor",
            Brew: "brew install phpactor",
        ),
        init_options: None,
        reads: &[
            ProfilePath::Home(".composer"),
            ProfilePath::Home(".config/composer"),
        ],
        writes: &[
            ProfilePath::Home(".cache/phpactor"),
            ProfilePath::Home(".local/share/phpactor"),
        ],
        experimental: false,
        default_enabled: true,
    },
];

pub struct ChosenInstall {
    pub chosen: Option<Install>,
    pub alternatives: Vec<Install>,
}

pub fn choose_install(entry: &ServerEntry, path_dirs: &[PathBuf]) -> ChosenInstall {
    let mut ranked: Vec<&Install> = entry.install.iter().collect();
    ranked.sort_by_key(|i| (!i.manager.available(path_dirs), i.manager.rank()));
    let chosen = ranked
        .iter()
        .find(|i| i.manager.available(path_dirs))
        .or_else(|| ranked.first())
        .map(|i| **i);
    let alternatives: Vec<Install> = ranked
        .iter()
        .filter(|i| Some(i.manager) != chosen.map(|c| c.manager))
        .map(|i| **i)
        .collect();
    ChosenInstall {
        chosen,
        alternatives,
    }
}

fn file_name_lower(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    base.to_lowercase()
}

fn extension_lower(name: &str) -> Option<String> {
    let dot = name.rfind('.')?;
    if dot == 0 || dot + 1 >= name.len() {
        return None;
    }
    Some(name[dot + 1..].to_lowercase())
}

pub fn entry_for_file(path: &str) -> Option<(&'static ServerEntry, &'static str)> {
    let name = file_name_lower(path);
    for entry in REGISTRY {
        for (fname, lang) in entry.filenames {
            if name == *fname {
                return Some((entry, lang));
            }
        }
        if entry.dockerfile_prefix && name.starts_with("dockerfile.") {
            let lang = entry
                .extensions
                .first()
                .map(|(_, l)| *l)
                .unwrap_or("dockerfile");
            return Some((entry, lang));
        }
    }
    let ext = extension_lower(&name)?;
    for entry in REGISTRY {
        for (e, lang) in entry.extensions {
            if ext == *e {
                return Some((entry, lang));
            }
        }
    }
    None
}

pub fn entry_by_id(id: &str) -> Option<&'static ServerEntry> {
    REGISTRY.iter().find(|e| e.id == id)
}

fn extra_dir(dir: BinDir, home: &Path, root: &Path) -> Option<PathBuf> {
    Some(match dir {
        BinDir::HomeCargoBin => home.join(".cargo/bin"),
        BinDir::HomeGoBin => home.join("go/bin"),
        BinDir::HomeLocalBin => home.join(".local/bin"),
        BinDir::NodeModulesBin => root.join("node_modules/.bin"),
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
        || ["exe", "cmd", "bat"]
            .iter()
            .any(|ext| path.with_extension(ext).is_file())
}

#[cfg(unix)]
fn candidate_names(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

#[cfg(not(unix))]
fn candidate_names(name: &str) -> Vec<String> {
    vec![
        name.to_string(),
        format!("{name}.exe"),
        format!("{name}.cmd"),
        format!("{name}.bat"),
    ]
}

fn find_in_dirs(names: &[&str], dirs: &[PathBuf]) -> Option<PathBuf> {
    for bin in names {
        for dir in dirs {
            for candidate in candidate_names(bin) {
                let full = dir.join(&candidate);
                if is_executable(&full) {
                    return Some(full);
                }
            }
        }
    }
    None
}

pub fn discover(
    entry: &ServerEntry,
    path_dirs: &[PathBuf],
    home: &Path,
    root: &Path,
) -> Option<PathBuf> {
    let mut search: Vec<PathBuf> = path_dirs.to_vec();
    for dir in entry.extra_bin_dirs {
        if let Some(resolved) = extra_dir(*dir, home, root) {
            search.push(resolved);
        }
    }
    find_in_dirs(entry.binaries, &search)
}

pub fn resolve_real(binary: &Path) -> PathBuf {
    std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf())
}

pub fn node_modules_root(real: &Path) -> Option<PathBuf> {
    let mut last: Option<PathBuf> = None;
    for ancestor in real.ancestors() {
        if ancestor
            .file_name()
            .map(|n| n == "node_modules")
            .unwrap_or(false)
        {
            last = Some(ancestor.to_path_buf());
        }
    }
    last
}

pub fn discover_interpreter(runtime: Runtime, path_dirs: &[PathBuf]) -> Option<PathBuf> {
    let name = runtime.interpreter()?;
    find_in_dirs(&[name], path_dirs)
}

fn colocated_node(real_binary: &Path) -> Option<PathBuf> {
    let nm = node_modules_root(real_binary)?;
    let mut prefix = nm.parent()?;
    if prefix.file_name().map(|n| n == "lib").unwrap_or(false) {
        prefix = prefix.parent()?;
    }
    let node = prefix.join("bin").join("node");
    is_executable(&node).then(|| resolve_real(&node))
}

pub fn resolve_interpreter(
    entry: &ServerEntry,
    binary: &Path,
    path_dirs: &[PathBuf],
) -> Option<PathBuf> {
    if entry.runtime == Runtime::Native {
        return None;
    }
    if entry.runtime == Runtime::Node {
        if let Some(node) = colocated_node(&resolve_real(binary)) {
            return Some(node);
        }
    }
    discover_interpreter(entry.runtime, path_dirs).map(|p| resolve_real(&p))
}

pub fn spawn_command(
    entry: &ServerEntry,
    binary: &Path,
    path_dirs: &[PathBuf],
) -> (PathBuf, Vec<PathBuf>) {
    if entry.runtime == Runtime::Node {
        if let Some(node) = resolve_interpreter(entry, binary, path_dirs) {
            return (node, vec![resolve_real(binary)]);
        }
    }
    (binary.to_path_buf(), Vec::new())
}

pub fn derived_read_grants(
    entry: &ServerEntry,
    binary: &Path,
    path_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let mut grants: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let push = |p: PathBuf, grants: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>| {
        if seen.insert(p.clone()) {
            grants.push(p);
        }
    };

    let real = resolve_real(binary);
    match node_modules_root(&real) {
        Some(nm) => push(nm, &mut grants, &mut seen),
        None => {
            if let Some(parent) = real.parent() {
                push(parent.to_path_buf(), &mut grants, &mut seen);
            }
        }
    }

    if let Some(interp) = resolve_interpreter(entry, binary, path_dirs) {
        push(install_prefix(&interp), &mut grants, &mut seen);
        push(interp, &mut grants, &mut seen);
    }

    grants
}

fn install_prefix(binary: &Path) -> PathBuf {
    let parent = binary.parent().unwrap_or(binary);
    if parent.file_name().map(|n| n == "bin").unwrap_or(false) {
        parent.parent().unwrap_or(parent).to_path_buf()
    } else {
        parent.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_by_extension() {
        let (entry, lang) = entry_for_file("src/main.rs").unwrap();
        assert_eq!(entry.id, "rust-analyzer");
        assert_eq!(lang, "rust");
    }

    #[test]
    fn resolves_by_bare_filename() {
        let (entry, lang) = entry_for_file("deploy/Dockerfile").unwrap();
        assert_eq!(entry.id, "dockerfile-language-server");
        assert_eq!(lang, "dockerfile");
    }

    #[test]
    fn resolves_dockerfile_with_a_suffix() {
        let (entry, _) = entry_for_file("Dockerfile.prod").unwrap();
        assert_eq!(entry.id, "dockerfile-language-server");
    }

    #[test]
    fn typescript_family_maps_extension_to_the_right_language_id() {
        assert_eq!(entry_for_file("a.ts").unwrap().1, "typescript");
        assert_eq!(entry_for_file("a.tsx").unwrap().1, "typescriptreact");
        assert_eq!(entry_for_file("a.jsx").unwrap().1, "javascriptreact");
        let (entry, _) = entry_for_file("a.tsx").unwrap();
        assert_eq!(entry.id, "typescript-language-server");
    }

    #[test]
    fn unknown_extension_has_no_server() {
        assert!(entry_for_file("photo.xyz").is_none());
        assert!(entry_for_file("noext").is_none());
    }

    #[test]
    fn registry_has_fifteen_unique_servers() {
        assert_eq!(REGISTRY.len(), 15);
        let mut ids: Vec<&str> = REGISTRY.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 15, "ids de server precisam ser únicos");
    }

    #[test]
    fn every_server_offers_at_least_one_install() {
        for entry in REGISTRY {
            assert!(!entry.install.is_empty(), "{} sem install", entry.id);
        }
    }

    #[test]
    fn jdtls_is_experimental_and_off_by_default() {
        let jdtls = entry_by_id("jdtls").unwrap();
        assert!(jdtls.experimental);
        assert!(!jdtls.default_enabled);
    }

    #[test]
    fn discovery_finds_a_binary_on_a_synthetic_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("rust-analyzer");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let entry = entry_by_id("rust-analyzer").unwrap();
        let found = discover(
            entry,
            &[dir.path().to_path_buf()],
            Path::new("/nonexistent-home"),
            Path::new("/nonexistent-root"),
        );
        assert_eq!(found.as_deref(), Some(bin.as_path()));
    }

    #[test]
    fn discovery_searches_project_node_modules_bin() {
        let root = tempfile::tempdir().unwrap();
        let bindir = root.path().join("node_modules/.bin");
        std::fs::create_dir_all(&bindir).unwrap();
        let bin = bindir.join("typescript-language-server");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let entry = entry_by_id("typescript-language-server").unwrap();
        let found = discover(entry, &[], Path::new("/nonexistent-home"), root.path());
        assert_eq!(found.as_deref(), Some(bin.as_path()));
    }

    #[test]
    fn discovery_ignores_a_non_executable_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gopls"), "not runnable").unwrap();
        let entry = entry_by_id("gopls").unwrap();
        let found = discover(
            entry,
            &[dir.path().to_path_buf()],
            Path::new("/nonexistent-home"),
            Path::new("/nonexistent-root"),
        );
        #[cfg(unix)]
        assert!(found.is_none(), "arquivo sem bit de execução não conta");
        let _ = found;
    }

    #[cfg(unix)]
    #[test]
    fn derived_grants_include_the_node_modules_tree_of_a_symlinked_wrapper() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let prefix = tempfile::tempdir().unwrap();
        let pkg = prefix
            .path()
            .join("lib/node_modules/typescript-language-server/lib");
        std::fs::create_dir_all(&pkg).unwrap();
        let cli = pkg.join("cli.mjs");
        std::fs::write(&cli, "#!/usr/bin/env node\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let bindir = prefix.path().join("bin");
        std::fs::create_dir_all(&bindir).unwrap();
        let wrapper = bindir.join("typescript-language-server");
        symlink(&cli, &wrapper).unwrap();

        let entry = entry_by_id("typescript-language-server").unwrap();
        let grants = derived_read_grants(entry, &wrapper, &[]);
        let node_modules = prefix
            .path()
            .join("lib/node_modules")
            .canonicalize()
            .unwrap();
        assert!(
            grants.contains(&node_modules),
            "a árvore node_modules inteira (deps hoisted) precisa ser legível: {grants:?}"
        );
    }

    #[test]
    fn install_prefers_a_present_manager_by_rank() {
        let entry = entry_by_id("typescript-language-server").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let bun = dir.path().join("bun");
        std::fs::write(&bun, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bun, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let chosen = choose_install(entry, &[dir.path().to_path_buf()]);
        assert_eq!(chosen.chosen.unwrap().manager, Manager::Bun);
        assert!(chosen
            .alternatives
            .iter()
            .any(|i| i.manager == Manager::Npm));
    }

    #[test]
    fn forwarded_env_collects_profile_env_vars() {
        let gopls = entry_by_id("gopls").unwrap();
        let vars = gopls.forwarded_env();
        assert!(vars.contains(&"GOPATH"));
        assert!(vars.contains(&"GOMODCACHE"));
        assert!(vars.contains(&"GOCACHE"));
    }

    #[test]
    fn gopls_write_grant_is_the_module_cache_not_the_gopath_bin() {
        let gopls = entry_by_id("gopls").unwrap();
        let writes: Vec<&'static str> = gopls
            .writes
            .iter()
            .filter_map(|p| match p {
                ProfilePath::Env(v, _) => Some(*v),
                _ => None,
            })
            .collect();
        assert!(writes.contains(&"GOMODCACHE"), "escreve o module cache");
        assert!(
            !writes.contains(&"GOPATH"),
            "não escreve GOPATH inteiro (que contém go/bin no PATH)"
        );
    }
}
