use super::registry::Platform;
use super::registry::{Archive, NpmPin, Pin, PlatformPin};

pub(super) static NODE_RUNTIME: &[PlatformPin] = &[
    PlatformPin {
        platform: Platform::LinuxX86_64,
        pin: Pin {
            version: "24.18.0",
            url: "https://nodejs.org/dist/v24.18.0/node-v24.18.0-linux-x64.tar.gz",
            sha256: "783130984963db7ba9cbd01089eaf2c2efb055c7c1693c943174b967b3050cb8",
            size: 57224421,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
    PlatformPin {
        platform: Platform::LinuxAarch64,
        pin: Pin {
            version: "24.18.0",
            url: "https://nodejs.org/dist/v24.18.0/node-v24.18.0-linux-arm64.tar.gz",
            sha256: "6b4484c2190274175df9aa8f28e2d758a819cb1c1fe6ab481e2f95b463ab8508",
            size: 56979089,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
    PlatformPin {
        platform: Platform::MacosAarch64,
        pin: Pin {
            version: "24.18.0",
            url: "https://nodejs.org/dist/v24.18.0/node-v24.18.0-darwin-arm64.tar.gz",
            sha256: "e1a97e14c99c803e96c7339403282ea05a499c32f8d83defe9ef5ec66f979ed1",
            size: 52087559,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
    PlatformPin {
        platform: Platform::MacosX86_64,
        pin: Pin {
            version: "24.18.0",
            url: "https://nodejs.org/dist/v24.18.0/node-v24.18.0-darwin-x64.tar.gz",
            sha256: "dfd0dbd3e721503434df7b7205e719f61b3a3a31b2bcf9729b8b91fea240f080",
            size: 53282687,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
];

pub(super) static RUST_ANALYZER: &[PlatformPin] = &[
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "2026-07-20", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-07-20/rust-analyzer-x86_64-unknown-linux-gnu.gz", sha256: "d12f8e6df9b6d84373e80cddb67d183587a52323878e87f4fb6df91814c23d80", size: 15028892, archive: Archive::Gzip, member: "rust-analyzer" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "2026-07-20", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-07-20/rust-analyzer-aarch64-unknown-linux-gnu.gz", sha256: "ff03f51db39f67ce5694fed091ca38f936195e5079d26f6c9df64596b5c9e640", size: 14439905, archive: Archive::Gzip, member: "rust-analyzer" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "2026-07-20", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-07-20/rust-analyzer-aarch64-apple-darwin.gz", sha256: "f0db74fed7e356c987a319bde43937e36d3248fc97b390c25abf972e2076b022", size: 13988226, archive: Archive::Gzip, member: "rust-analyzer" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "2026-07-20", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-07-20/rust-analyzer-x86_64-apple-darwin.gz", sha256: "2af82b65570becf6aeebce5a891a4724074c0857803dfa46f891487e06218c96", size: 14709095, archive: Archive::Gzip, member: "rust-analyzer" } },
];

pub(super) static TAPLO: &[PlatformPin] = &[
    PlatformPin {
        platform: Platform::LinuxX86_64,
        pin: Pin {
            version: "0.10.0",
            url: "https://github.com/tamasfe/taplo/releases/download/0.10.0/taplo-linux-x86_64.gz",
            sha256: "8fe196b894ccf9072f98d4e1013a180306e17d244830b03986ee5e8eabeb6156",
            size: 5116068,
            archive: Archive::Gzip,
            member: "taplo",
        },
    },
    PlatformPin {
        platform: Platform::LinuxAarch64,
        pin: Pin {
            version: "0.10.0",
            url: "https://github.com/tamasfe/taplo/releases/download/0.10.0/taplo-linux-aarch64.gz",
            sha256: "033681d01eec8376c3fd38fa3703c79316f5e14bb013d859943b60a07bccdcc3",
            size: 4631779,
            archive: Archive::Gzip,
            member: "taplo",
        },
    },
    PlatformPin {
        platform: Platform::MacosAarch64,
        pin: Pin {
            version: "0.10.0",
            url:
                "https://github.com/tamasfe/taplo/releases/download/0.10.0/taplo-darwin-aarch64.gz",
            sha256: "713734314c3e71894b9e77513c5349835eefbd52908445a0d73b0c7dc469347d",
            size: 4616415,
            archive: Archive::Gzip,
            member: "taplo",
        },
    },
    PlatformPin {
        platform: Platform::MacosX86_64,
        pin: Pin {
            version: "0.10.0",
            url: "https://github.com/tamasfe/taplo/releases/download/0.10.0/taplo-darwin-x86_64.gz",
            sha256: "898122cde3a0b1cd1cbc2d52d3624f23338218c91b5ddb71518236a4c2c10ef2",
            size: 4921954,
            archive: Archive::Gzip,
            member: "taplo",
        },
    },
];

pub(super) static MARKSMAN: &[PlatformPin] = &[
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "2026-02-08", url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman-linux-x64", sha256: "be5098e8213219269c47fc0d916a66fa31ce0602ec967475c722260aabf26087", size: 22500875, archive: Archive::Raw, member: "marksman" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "2026-02-08", url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman-linux-arm64", sha256: "db8e124527f7f8048e3e6c91821b9c52ef173d92c01e47d221bf1337afd962fb", size: 21851058, archive: Archive::Raw, member: "marksman" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "2026-02-08", url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman-macos", sha256: "6a801c17b5ac0dba69787c5282b3b3bd416e66c96253fae098d311c6bbd1833b", size: 43856208, archive: Archive::Raw, member: "marksman" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "2026-02-08", url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman-macos", sha256: "6a801c17b5ac0dba69787c5282b3b3bd416e66c96253fae098d311c6bbd1833b", size: 43856208, archive: Archive::Raw, member: "marksman" } },
];

pub(super) static TERRAFORM_LS: &[PlatformPin] = &[
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "0.38.8", url: "https://releases.hashicorp.com/terraform-ls/0.38.8/terraform-ls_0.38.8_linux_amd64.zip", sha256: "d16077d9c83f13ac33501af49ea75f43218d3fa2437c6c1374550b2625edc3ef", size: 30326575, archive: Archive::Zip, member: "terraform-ls" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "0.38.8", url: "https://releases.hashicorp.com/terraform-ls/0.38.8/terraform-ls_0.38.8_linux_arm64.zip", sha256: "762db754428dd188b949533ca05437955e26f4b3fc699d4b93392668a24e7a10", size: 29620863, archive: Archive::Zip, member: "terraform-ls" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "0.38.8", url: "https://releases.hashicorp.com/terraform-ls/0.38.8/terraform-ls_0.38.8_darwin_arm64.zip", sha256: "510a506f7bf1550294202347261961e52daa4664a795e2deffbf7df7296b1f6c", size: 30012654, archive: Archive::Zip, member: "terraform-ls" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "0.38.8", url: "https://releases.hashicorp.com/terraform-ls/0.38.8/terraform-ls_0.38.8_darwin_amd64.zip", sha256: "34cfe6cbbb61da5b8fd21721e14be0f134417f249350872da1669454dc8762a4", size: 30709588, archive: Archive::Zip, member: "terraform-ls" } },
];

pub(super) static CLOJURE_LSP: &[PlatformPin] = &[
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "2026.07.06-14.34.19", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.07.06-14.34.19/clojure-lsp-native-linux-amd64.zip", sha256: "520f724ee02f4b3ecb225395a7a5a4ccad3878d6d1418240cd9636afcf9b858e", size: 35530120, archive: Archive::Zip, member: "clojure-lsp" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "2026.07.06-14.34.19", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.07.06-14.34.19/clojure-lsp-native-linux-aarch64.zip", sha256: "0595e65a5934d3208246f529b5cf0497d7167d7e9b8317e9b391e05b5c0906d7", size: 43535726, archive: Archive::Zip, member: "clojure-lsp" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "2026.07.06-14.34.19", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.07.06-14.34.19/clojure-lsp-native-macos-aarch64.zip", sha256: "dd9a8e36add53b8d8166bb3d7580c6e5563401aea87b62600786af2e7d37ccde", size: 40569964, archive: Archive::Zip, member: "clojure-lsp" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "2026.07.06-14.34.19", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.07.06-14.34.19/clojure-lsp-native-macos-amd64.zip", sha256: "0449f7f8fc975157cb4e5cdcf365bcd43bcf1fa47b99256427e7a86e4c17fc3f", size: 39673366, archive: Archive::Zip, member: "clojure-lsp" } },
];

pub(super) static TYPESCRIPT_LANGUAGE_SERVER: &[NpmPin] = &[
    NpmPin { package: "typescript-language-server", pin: Pin { version: "5.3.0", url: "https://registry.npmjs.org/typescript-language-server/-/typescript-language-server-5.3.0.tgz", sha256: "398cacc17fff2108652e7b4050e3182008d17063246b3fea7dcf5fae2ce1560e", size: 501633, archive: Archive::TarGz, member: "" } },
    NpmPin { package: "typescript", pin: Pin { version: "5.9.3", url: "https://registry.npmjs.org/typescript/-/typescript-5.9.3.tgz", sha256: "10e108c9cf7d5f2879053dff18515fb405abf2ccef63eaaf017d9c571687a1d3", size: 4377468, archive: Archive::TarGz, member: "" } },
];

pub(super) static PYRIGHT: &[NpmPin] = &[NpmPin {
    package: "pyright",
    pin: Pin {
        version: "1.1.411",
        url: "https://registry.npmjs.org/pyright/-/pyright-1.1.411.tgz",
        sha256: "bd5c488fc20fa237a944279bf32cae2f986cf10d5d5d9e8705819859daeb2f4a",
        size: 4139958,
        archive: Archive::TarGz,
        member: "",
    },
}];
