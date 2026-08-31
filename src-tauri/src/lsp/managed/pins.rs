use super::registry::Platform;
use super::registry::{Archive, NpmPin, Pin, PlatformPin};

pub(super) static NODE_RUNTIME: &[PlatformPin] = &[
    PlatformPin {
        platform: Platform::LinuxX86_64,
        pin: Pin {
            version: "24.20.0",
            url: "https://nodejs.org/dist/v24.20.0/node-v24.20.0-linux-x64.tar.gz",
            sha256: "855d581f8a4eb1a8117e3426de25fe02770592febcfb31369aee1ffbfee9e8ec",
            size: 58006679,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
    PlatformPin {
        platform: Platform::LinuxAarch64,
        pin: Pin {
            version: "24.20.0",
            url: "https://nodejs.org/dist/v24.20.0/node-v24.20.0-linux-arm64.tar.gz",
            sha256: "3515603e2487879a39bc75716f1a2affd027500c64ba50e845cf72cb33219013",
            size: 57732896,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
    PlatformPin {
        platform: Platform::MacosAarch64,
        pin: Pin {
            version: "24.20.0",
            url: "https://nodejs.org/dist/v24.20.0/node-v24.20.0-darwin-arm64.tar.gz",
            sha256: "40e5607e5ecb3db9192723776da2d75d966260fc74a7a9e731c1bd67dda96bc8",
            size: 52813331,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
    PlatformPin {
        platform: Platform::MacosX86_64,
        pin: Pin {
            version: "24.20.0",
            url: "https://nodejs.org/dist/v24.20.0/node-v24.20.0-darwin-x64.tar.gz",
            sha256: "9e5b2644cf107befb6aefca676b96d3296bc10138096f022ed378d6233ed81f4",
            size: 54021618,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
];

pub(super) static RUST_ANALYZER: &[PlatformPin] = &[
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "2026-08-31", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-08-31/rust-analyzer-x86_64-unknown-linux-gnu.gz", sha256: "a775f9edb543bb0c077da339b783e89446092986cde921c9d8e1733b091080c6", size: 14879817, archive: Archive::Gzip, member: "rust-analyzer" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "2026-08-31", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-08-31/rust-analyzer-aarch64-unknown-linux-gnu.gz", sha256: "d7617a9a7760cb615ae2d6e3b3a9456a0719959daad766fdda45bf58ec13661d", size: 14330737, archive: Archive::Gzip, member: "rust-analyzer" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "2026-08-31", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-08-31/rust-analyzer-aarch64-apple-darwin.gz", sha256: "5de5c20b8b49bdc9339ef537b4029af7f97214fb88d6fc86af57ab9344467625", size: 13883227, archive: Archive::Gzip, member: "rust-analyzer" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "2026-08-31", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-08-31/rust-analyzer-x86_64-apple-darwin.gz", sha256: "a3b3e06da42b4df781891359ccbb57d6a84f311f0506de5f98dd0553460b1f8d", size: 14603811, archive: Archive::Gzip, member: "rust-analyzer" } },
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
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "0.39.0", url: "https://releases.hashicorp.com/terraform-ls/0.39.0/terraform-ls_0.39.0_linux_amd64.zip", sha256: "7750edc736845fd8c04ff0fc6332423c12d8275b358668c8c17e8aedc43ef971", size: 31026533, archive: Archive::Zip, member: "terraform-ls" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "0.39.0", url: "https://releases.hashicorp.com/terraform-ls/0.39.0/terraform-ls_0.39.0_linux_arm64.zip", sha256: "62f32ea22cb78e5e5667ed638ad6e0fbde30ab59228d073c3c9bb249f89c7f5a", size: 30305656, archive: Archive::Zip, member: "terraform-ls" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "0.39.0", url: "https://releases.hashicorp.com/terraform-ls/0.39.0/terraform-ls_0.39.0_darwin_arm64.zip", sha256: "6f80fe0b34af184175508f3d9135d8159f5dce4000d9b39540553eb1c267c54b", size: 30705654, archive: Archive::Zip, member: "terraform-ls" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "0.39.0", url: "https://releases.hashicorp.com/terraform-ls/0.39.0/terraform-ls_0.39.0_darwin_amd64.zip", sha256: "cc5bbc5b5a39d12d455c0d2b1e4b3a2c1f237d02d2cf819cf5252358f2d674de", size: 31418012, archive: Archive::Zip, member: "terraform-ls" } },
];

pub(super) static CLOJURE_LSP: &[PlatformPin] = &[
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "2026.07.06-14.34.19", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.07.06-14.34.19/clojure-lsp-native-linux-amd64.zip", sha256: "520f724ee02f4b3ecb225395a7a5a4ccad3878d6d1418240cd9636afcf9b858e", size: 35530120, archive: Archive::Zip, member: "clojure-lsp" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "2026.07.06-14.34.19", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.07.06-14.34.19/clojure-lsp-native-linux-aarch64.zip", sha256: "0595e65a5934d3208246f529b5cf0497d7167d7e9b8317e9b391e05b5c0906d7", size: 43535726, archive: Archive::Zip, member: "clojure-lsp" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "2026.07.06-14.34.19", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.07.06-14.34.19/clojure-lsp-native-macos-aarch64.zip", sha256: "dd9a8e36add53b8d8166bb3d7580c6e5563401aea87b62600786af2e7d37ccde", size: 40569964, archive: Archive::Zip, member: "clojure-lsp" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "2026.07.06-14.34.19", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.07.06-14.34.19/clojure-lsp-native-macos-amd64.zip", sha256: "0449f7f8fc975157cb4e5cdcf365bcd43bcf1fa47b99256427e7a86e4c17fc3f", size: 39673366, archive: Archive::Zip, member: "clojure-lsp" } },
];

pub(super) static TYPESCRIPT_LANGUAGE_SERVER: &[NpmPin] = &[
    NpmPin { package: "typescript-language-server", pin: Pin { version: "6.0.0", url: "https://registry.npmjs.org/typescript-language-server/-/typescript-language-server-6.0.0.tgz", sha256: "6e23b48efc76af4e70928cdfe62ea6e6cfef67ab4c1e7579c4e82dd284fbdfd2", size: 515598, archive: Archive::TarGz, member: "" } },
    NpmPin { package: "typescript", pin: Pin { version: "5.9.3", url: "https://registry.npmjs.org/typescript/-/typescript-5.9.3.tgz", sha256: "10e108c9cf7d5f2879053dff18515fb405abf2ccef63eaaf017d9c571687a1d3", size: 4377468, archive: Archive::TarGz, member: "" } },
];

pub(super) static PYRIGHT: &[NpmPin] = &[NpmPin {
    package: "pyright",
    pin: Pin {
        version: "1.1.413",
        url: "https://registry.npmjs.org/pyright/-/pyright-1.1.413.tgz",
        sha256: "7322a75188e788f9fe7cbb71891af435a713bf8985141dc0d28e8ca243977bee",
        size: 4155725,
        archive: Archive::TarGz,
        member: "",
    },
}];
