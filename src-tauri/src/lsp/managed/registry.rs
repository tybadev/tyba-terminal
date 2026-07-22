use super::pins;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    #[serde(rename = "linux-x86_64")]
    LinuxX86_64,
    #[serde(rename = "linux-aarch64")]
    LinuxAarch64,
    #[serde(rename = "macos-aarch64")]
    MacosAarch64,
    #[serde(rename = "macos-x86_64")]
    MacosX86_64,
}

impl Platform {
    pub fn current() -> Option<Platform> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            Some(Platform::LinuxX86_64)
        }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        {
            Some(Platform::LinuxAarch64)
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            Some(Platform::MacosAarch64)
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            Some(Platform::MacosX86_64)
        }
        #[cfg(not(any(
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
        )))]
        {
            None
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Platform::LinuxX86_64 => "linux-x86_64",
            Platform::LinuxAarch64 => "linux-aarch64",
            Platform::MacosAarch64 => "macos-aarch64",
            Platform::MacosX86_64 => "macos-x86_64",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    Gzip,
    Zip,
    TarGz,
    Raw,
}

pub struct Pin {
    pub version: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub archive: Archive,
    pub member: &'static str,
}

pub struct PlatformPin {
    pub platform: Platform,
    pub pin: Pin,
}

pub struct NpmPin {
    pub package: &'static str,
    pub pin: Pin,
}

pub enum Source {
    Native {
        pins: &'static [PlatformPin],
    },
    Node {
        packages: &'static [NpmPin],
        entry: &'static str,
    },
}

pub struct Managed {
    pub id: &'static str,
    pub source: Source,
}

pub static MANAGED: &[Managed] = &[
    Managed {
        id: "rust-analyzer",
        source: Source::Native {
            pins: pins::RUST_ANALYZER,
        },
    },
    Managed {
        id: "taplo",
        source: Source::Native { pins: pins::TAPLO },
    },
    Managed {
        id: "marksman",
        source: Source::Native {
            pins: pins::MARKSMAN,
        },
    },
    Managed {
        id: "terraform-ls",
        source: Source::Native {
            pins: pins::TERRAFORM_LS,
        },
    },
    Managed {
        id: "clojure-lsp",
        source: Source::Native {
            pins: pins::CLOJURE_LSP,
        },
    },
    Managed {
        id: "typescript-language-server",
        source: Source::Node {
            packages: pins::TYPESCRIPT_LANGUAGE_SERVER,
            entry: "typescript-language-server/lib/cli.mjs",
        },
    },
    Managed {
        id: "pyright",
        source: Source::Node {
            packages: pins::PYRIGHT,
            entry: "pyright/langserver.index.js",
        },
    },
];

pub fn managed_for(id: &str) -> Option<&'static Managed> {
    MANAGED.iter().find(|m| m.id == id)
}

impl Managed {
    pub fn version(&self) -> &'static str {
        match &self.source {
            Source::Native { pins } => pins.first().map(|p| p.pin.version).unwrap_or(""),
            Source::Node { packages, .. } => packages.first().map(|p| p.pin.version).unwrap_or(""),
        }
    }

    pub fn is_node(&self) -> bool {
        matches!(self.source, Source::Node { .. })
    }

    pub fn native_pin(&self, platform: Platform) -> Option<&'static Pin> {
        match &self.source {
            Source::Native { pins } => pins.iter().find(|p| p.platform == platform).map(|p| &p.pin),
            Source::Node { .. } => None,
        }
    }

    pub fn npm_packages(&self) -> &'static [NpmPin] {
        match &self.source {
            Source::Node { packages, .. } => packages,
            Source::Native { .. } => &[],
        }
    }

    pub fn node_entry(&self) -> Option<&'static str> {
        match &self.source {
            Source::Node { entry, .. } => Some(entry),
            Source::Native { .. } => None,
        }
    }

    pub fn available_for(&self, platform: Platform) -> bool {
        match &self.source {
            Source::Native { .. } => self.native_pin(platform).is_some(),
            Source::Node { .. } => node_runtime_pin(platform).is_some(),
        }
    }

    pub fn download_size(&self, platform: Platform) -> u64 {
        match &self.source {
            Source::Native { .. } => self.native_pin(platform).map(|p| p.size).unwrap_or(0),
            Source::Node { packages, .. } => {
                let deps: u64 = packages.iter().map(|p| p.pin.size).sum();
                let runtime = node_runtime_pin(platform).map(|p| p.size).unwrap_or(0);
                deps + runtime
            }
        }
    }

    pub fn source_host(&self) -> &'static str {
        let url = match &self.source {
            Source::Native { pins } => pins.first().map(|p| p.pin.url).unwrap_or(""),
            Source::Node { packages, .. } => packages.first().map(|p| p.pin.url).unwrap_or(""),
        };
        url_host(url)
    }
}

pub fn node_runtime_pin(platform: Platform) -> Option<&'static Pin> {
    pins::NODE_RUNTIME
        .iter()
        .find(|p| p.platform == platform)
        .map(|p| &p.pin)
}

pub fn node_runtime_version() -> &'static str {
    pins::NODE_RUNTIME
        .first()
        .map(|p| p.pin.version)
        .unwrap_or("")
}

fn url_host(url: &str) -> &str {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    match rest {
        Some(rest) => rest.split('/').next().unwrap_or(rest),
        None => url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eleven_managed_servers_split_five_native_plus_node() {
        assert_eq!(MANAGED.len(), 7);
        let native = MANAGED
            .iter()
            .filter(|m| matches!(m.source, Source::Native { .. }))
            .count();
        let node = MANAGED
            .iter()
            .filter(|m| matches!(m.source, Source::Node { .. }))
            .count();
        assert_eq!(native, 5, "5 servers nativos gerenciados");
        assert_eq!(
            node, 2,
            "typescript-language-server e pyright são self-contained"
        );
    }

    #[test]
    fn every_native_server_pins_all_four_platforms() {
        for m in MANAGED {
            if let Source::Native { pins } = &m.source {
                for platform in [
                    Platform::LinuxX86_64,
                    Platform::LinuxAarch64,
                    Platform::MacosAarch64,
                    Platform::MacosX86_64,
                ] {
                    assert!(
                        pins.iter().any(|p| p.platform == platform),
                        "{} sem pin para {}",
                        m.id,
                        platform.id()
                    );
                }
            }
        }
    }

    #[test]
    fn node_runtime_covers_all_four_platforms() {
        for platform in [
            Platform::LinuxX86_64,
            Platform::LinuxAarch64,
            Platform::MacosAarch64,
            Platform::MacosX86_64,
        ] {
            assert!(
                node_runtime_pin(platform).is_some(),
                "node sem pin {}",
                platform.id()
            );
        }
    }

    #[test]
    fn managed_ids_match_the_lsp_registry() {
        for m in MANAGED {
            assert!(
                crate::lsp::registry::entry_by_id(m.id).is_some(),
                "{} não existe no registry de LSP",
                m.id
            );
        }
    }

    #[test]
    fn typescript_server_pins_its_typescript_companion() {
        let tsls = managed_for("typescript-language-server").unwrap();
        let pkgs: Vec<&str> = tsls.npm_packages().iter().map(|p| p.package).collect();
        assert!(pkgs.contains(&"typescript-language-server"));
        assert!(
            pkgs.contains(&"typescript"),
            "tsserver vem do pacote typescript"
        );
    }

    #[test]
    fn every_pin_is_https_and_carries_a_full_sha256() {
        for m in MANAGED {
            let pins: Vec<&Pin> = match &m.source {
                Source::Native { pins } => pins.iter().map(|p| &p.pin).collect(),
                Source::Node { packages, .. } => packages.iter().map(|p| &p.pin).collect(),
            };
            for pin in pins {
                assert!(pin.url.starts_with("https://"), "{} pin não-HTTPS", m.id);
                assert_eq!(pin.sha256.len(), 64, "{} sha256 malformado", m.id);
                assert!(pin.size > 0, "{} size zero", m.id);
            }
        }
        for p in pins::NODE_RUNTIME {
            assert!(p.pin.url.starts_with("https://"));
            assert_eq!(p.pin.sha256.len(), 64);
        }
    }

    #[test]
    fn source_host_is_the_official_domain() {
        assert_eq!(
            managed_for("rust-analyzer").unwrap().source_host(),
            "github.com"
        );
        assert_eq!(
            managed_for("terraform-ls").unwrap().source_host(),
            "releases.hashicorp.com"
        );
        assert_eq!(
            managed_for("typescript-language-server")
                .unwrap()
                .source_host(),
            "registry.npmjs.org"
        );
    }
}
