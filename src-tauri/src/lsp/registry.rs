use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Macos,
    Linux,
    Windows,
}

pub fn current_platform() -> Platform {
    #[cfg(target_os = "macos")]
    {
        Platform::Macos
    }
    #[cfg(target_os = "linux")]
    {
        Platform::Linux
    }
    #[cfg(target_os = "windows")]
    {
        Platform::Windows
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Platform::Linux
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InstallHint {
    pub platform: Platform,
    pub command: &'static str,
    pub manager: &'static str,
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

pub struct ServerEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub binaries: &'static [&'static str],
    pub args: &'static [&'static str],
    pub extensions: &'static [(&'static str, &'static str)],
    pub filenames: &'static [(&'static str, &'static str)],
    pub dockerfile_prefix: bool,
    pub extra_bin_dirs: &'static [BinDir],
    pub install: &'static [InstallHint],
    pub init_options: Option<&'static str>,
    pub reads: &'static [ProfilePath],
    pub writes: &'static [ProfilePath],
    pub experimental: bool,
    pub default_enabled: bool,
}

impl ServerEntry {
    pub fn install_hint(&self, platform: Platform) -> Option<&'static InstallHint> {
        self.install
            .iter()
            .find(|h| h.platform == platform)
            .or_else(|| self.install.first())
    }
}

macro_rules! hints {
    ($($plat:ident => $mgr:literal : $cmd:literal),+ $(,)?) => {
        &[$(InstallHint { platform: Platform::$plat, command: $cmd, manager: $mgr }),+]
    };
}

pub static REGISTRY: &[ServerEntry] = &[
    ServerEntry {
        id: "rust-analyzer",
        label: "rust-analyzer",
        binaries: &["rust-analyzer"],
        args: &[],
        extensions: &[("rs", "rust")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeCargoBin],
        install: hints!(
            Macos => "rustup": "rustup component add rust-analyzer",
            Linux => "rustup": "rustup component add rust-analyzer",
            Windows => "rustup": "rustup component add rust-analyzer",
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
        install: hints!(
            Macos => "npm": "npm install -g typescript-language-server typescript",
            Linux => "npm": "npm install -g typescript-language-server typescript",
            Windows => "npm": "npm install -g typescript-language-server typescript",
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
        binaries: &["pyright-langserver"],
        args: &["--stdio"],
        extensions: &[("py", "python"), ("pyi", "python")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: hints!(
            Macos => "npm": "npm install -g pyright",
            Linux => "npm": "npm install -g pyright",
            Windows => "npm": "npm install -g pyright",
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
        binaries: &["gopls"],
        args: &[],
        extensions: &[("go", "go")],
        filenames: &[("go.mod", "go.mod"), ("go.sum", "go.sum")],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeGoBin],
        install: hints!(
            Macos => "go": "go install golang.org/x/tools/gopls@latest",
            Linux => "go": "go install golang.org/x/tools/gopls@latest",
            Windows => "go": "go install golang.org/x/tools/gopls@latest",
        ),
        init_options: None,
        reads: &[ProfilePath::Env("GOPATH", "go")],
        writes: &[
            ProfilePath::Env("GOPATH", "go"),
            ProfilePath::Env("GOCACHE", "Library/Caches/go-build"),
            ProfilePath::Home(".cache/go-build"),
        ],
        experimental: false,
        default_enabled: true,
    },
    ServerEntry {
        id: "bash-language-server",
        label: "bash-language-server",
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
        install: hints!(
            Macos => "npm": "npm install -g bash-language-server",
            Linux => "npm": "npm install -g bash-language-server",
            Windows => "npm": "npm install -g bash-language-server",
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
        binaries: &["yaml-language-server"],
        args: &["--stdio"],
        extensions: &[("yaml", "yaml"), ("yml", "yaml")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: hints!(
            Macos => "npm": "npm install -g yaml-language-server",
            Linux => "npm": "npm install -g yaml-language-server",
            Windows => "npm": "npm install -g yaml-language-server",
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
        binaries: &["taplo"],
        args: &["lsp", "stdio"],
        extensions: &[("toml", "toml")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeCargoBin],
        install: hints!(
            Macos => "brew": "brew install taplo",
            Linux => "cargo": "cargo install taplo-cli --locked",
            Windows => "cargo": "cargo install taplo-cli --locked",
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
        binaries: &["docker-langserver"],
        args: &["--stdio"],
        extensions: &[("dockerfile", "dockerfile")],
        filenames: &[
            ("dockerfile", "dockerfile"),
            ("containerfile", "dockerfile"),
        ],
        dockerfile_prefix: true,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: hints!(
            Macos => "npm": "npm install -g dockerfile-language-server-nodejs",
            Linux => "npm": "npm install -g dockerfile-language-server-nodejs",
            Windows => "npm": "npm install -g dockerfile-language-server-nodejs",
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
        binaries: &["marksman"],
        args: &["server"],
        extensions: &[("md", "markdown"), ("markdown", "markdown")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: hints!(
            Macos => "brew": "brew install marksman",
            Linux => "manual": "curl -L -o ~/.local/bin/marksman https://github.com/artempyanykh/marksman/releases/latest/download/marksman-linux-x64 && chmod +x ~/.local/bin/marksman",
            Windows => "scoop": "scoop install marksman",
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
        binaries: &["vscode-json-language-server"],
        args: &["--stdio"],
        extensions: &[("json", "json"), ("jsonc", "jsonc")],
        filenames: &[("tsconfig.json", "jsonc"), (".babelrc", "jsonc")],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::NodeModulesBin],
        install: hints!(
            Macos => "npm": "npm install -g vscode-langservers-extracted",
            Linux => "npm": "npm install -g vscode-langservers-extracted",
            Windows => "npm": "npm install -g vscode-langservers-extracted",
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
        binaries: &["terraform-ls"],
        args: &["serve"],
        extensions: &[("tf", "terraform"), ("tfvars", "terraform")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: hints!(
            Macos => "brew": "brew install hashicorp/tap/terraform-ls",
            Linux => "brew": "brew install hashicorp/tap/terraform-ls",
            Windows => "scoop": "scoop install terraform-ls",
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
        binaries: &["jdtls"],
        args: &[],
        extensions: &[("java", "java")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: hints!(
            Macos => "brew": "brew install jdtls",
            Linux => "manual": "Baixe o Eclipse JDT LS e coloque `jdtls` no PATH",
            Windows => "manual": "Baixe o Eclipse JDT LS e coloque `jdtls` no PATH",
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
        install: hints!(
            Macos => "brew": "brew install clojure-lsp/brew/clojure-lsp-native",
            Linux => "manual": "Baixe clojure-lsp em github.com/clojure-lsp/clojure-lsp/releases",
            Windows => "scoop": "scoop install clojure-lsp",
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
        binaries: &["elixir-ls", "language_server.sh"],
        args: &[],
        extensions: &[("ex", "elixir"), ("exs", "elixir")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: hints!(
            Macos => "brew": "brew install elixir-ls",
            Linux => "manual": "Baixe ElixirLS em github.com/elixir-lsp/elixir-ls/releases",
            Windows => "manual": "Baixe ElixirLS em github.com/elixir-lsp/elixir-ls/releases",
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
        binaries: &["phpactor"],
        args: &["language-server"],
        extensions: &[("php", "php")],
        filenames: &[],
        dockerfile_prefix: false,
        extra_bin_dirs: &[BinDir::HomeLocalBin],
        install: hints!(
            Macos => "composer": "composer global require phpactor/phpactor",
            Linux => "composer": "composer global require phpactor/phpactor",
            Windows => "composer": "composer global require phpactor/phpactor",
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
    for bin in entry.binaries {
        for dir in &search {
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
    fn every_server_carries_an_install_hint_for_each_platform() {
        for entry in REGISTRY {
            for platform in [Platform::Macos, Platform::Linux, Platform::Windows] {
                let hint = entry.install_hint(platform).unwrap();
                assert!(
                    !hint.command.is_empty(),
                    "{} sem comando de instalação para {:?}",
                    entry.id,
                    platform
                );
            }
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
}
