use super::registry::Platform;
use super::registry::{Archive, NpmPin, Pin, PlatformPin};

pub(super) static NODE_RUNTIME: &[PlatformPin] = &[
    PlatformPin {
        platform: Platform::LinuxX86_64,
        pin: Pin {
            version: "26.13.0",
            url: "https://nodejs.org/dist/v26.13.0/node-v26.13.0-linux-x64.tar.gz",
            sha256: "5848553868b402d3fabb5b09270649977780a390f832155721025a08b5104455",
            size: 62893692,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
    PlatformPin {
        platform: Platform::LinuxAarch64,
        pin: Pin {
            version: "26.13.0",
            url: "https://nodejs.org/dist/v26.13.0/node-v26.13.0-linux-arm64.tar.gz",
            sha256: "439e9c438acdf6a6ffe47418da709153d66c4bed812e30a473faacb107dd2791",
            size: 62611009,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
    PlatformPin {
        platform: Platform::MacosAarch64,
        pin: Pin {
            version: "26.13.0",
            url: "https://nodejs.org/dist/v26.13.0/node-v26.13.0-darwin-arm64.tar.gz",
            sha256: "d38bd5e5a2ddce6bbb5ce45e46e208337da4d51fffe611e567fd32073d6439f6",
            size: 58334661,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
    PlatformPin {
        platform: Platform::MacosX86_64,
        pin: Pin {
            version: "26.13.0",
            url: "https://nodejs.org/dist/v26.13.0/node-v26.13.0-darwin-x64.tar.gz",
            sha256: "525c9b29bb0263d687de60a4cec0e2a6db876a8eb032369e5795eca71e7c358e",
            size: 59802926,
            archive: Archive::TarGz,
            member: "bin/node",
        },
    },
];

pub(super) static RUST_ANALYZER: &[PlatformPin] = &[
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "2026-11-09", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-11-09/rust-analyzer-x86_64-unknown-linux-gnu.gz", sha256: "8c039b25cb338b3970c731646677a4c9d4e9dd7a92c9ab9ae459c71467fbf4a9", size: 15661566, archive: Archive::Gzip, member: "rust-analyzer" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "2026-11-09", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-11-09/rust-analyzer-aarch64-unknown-linux-gnu.gz", sha256: "813ca3cec78ac6dab99c7bc9d51e63578f6d401b994ecf29cc0dcad86f9b2cd4", size: 15102920, archive: Archive::Gzip, member: "rust-analyzer" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "2026-11-09", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-11-09/rust-analyzer-aarch64-apple-darwin.gz", sha256: "e43a65008f8f4291320f33c0545dd577ad7623a7407b584a27155833d3722dfb", size: 14335571, archive: Archive::Gzip, member: "rust-analyzer" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "2026-11-09", url: "https://github.com/rust-lang/rust-analyzer/releases/download/2026-11-09/rust-analyzer-x86_64-apple-darwin.gz", sha256: "abf410a54e33e9f8b2d37f76dde8884db86a030d6a9cc70c0e3f87a77c084c5c", size: 15071019, archive: Archive::Gzip, member: "rust-analyzer" } },
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
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "0.39.1", url: "https://releases.hashicorp.com/terraform-ls/0.39.1/terraform-ls_0.39.1_linux_amd64.zip", sha256: "fe2a158e4a1a8ec8047dae9e06c439b468412aa579929ba28b2e1258cedef9c0", size: 34430497, archive: Archive::Zip, member: "terraform-ls" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "0.39.1", url: "https://releases.hashicorp.com/terraform-ls/0.39.1/terraform-ls_0.39.1_linux_arm64.zip", sha256: "a72a6d9f72580cfc30cc06d3c973fd5f55790f62e3ddb195998435070d9af36d", size: 33677998, archive: Archive::Zip, member: "terraform-ls" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "0.39.1", url: "https://releases.hashicorp.com/terraform-ls/0.39.1/terraform-ls_0.39.1_darwin_arm64.zip", sha256: "07748aa989fc3b9f6e4538ff83bc2e607ebc20c7587ce6556ae555d631e70b1b", size: 34111708, archive: Archive::Zip, member: "terraform-ls" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "0.39.1", url: "https://releases.hashicorp.com/terraform-ls/0.39.1/terraform-ls_0.39.1_darwin_amd64.zip", sha256: "25814afdbec9163de4ad8db58960195b9fc65ddc5405a5df7163bbdbd08b5325", size: 34858367, archive: Archive::Zip, member: "terraform-ls" } },
];

pub(super) static CLOJURE_LSP: &[PlatformPin] = &[
    PlatformPin { platform: Platform::LinuxX86_64, pin: Pin { version: "2026.10.15-11.42.38", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.10.15-11.42.38/clojure-lsp-native-linux-amd64.zip", sha256: "d68acf630465cec9d3685fd958f914d9228d90a07222edd301f1a4740b8e8b76", size: 35809024, archive: Archive::Zip, member: "clojure-lsp" } },
    PlatformPin { platform: Platform::LinuxAarch64, pin: Pin { version: "2026.10.15-11.42.38", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.10.15-11.42.38/clojure-lsp-native-linux-aarch64.zip", sha256: "232ac0cd191070a3b7b5532135ec9f97217b82324de7e97da0b1bf81353a3c94", size: 43862001, archive: Archive::Zip, member: "clojure-lsp" } },
    PlatformPin { platform: Platform::MacosAarch64, pin: Pin { version: "2026.10.15-11.42.38", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.10.15-11.42.38/clojure-lsp-native-macos-aarch64.zip", sha256: "84df57e51e27f4ecf20d0cd65aab3296e2ebf28dbaacca4fe216b6cf20485716", size: 40872630, archive: Archive::Zip, member: "clojure-lsp" } },
    PlatformPin { platform: Platform::MacosX86_64, pin: Pin { version: "2026.10.15-11.42.38", url: "https://github.com/clojure-lsp/clojure-lsp/releases/download/2026.10.15-11.42.38/clojure-lsp-native-macos-amd64.zip", sha256: "468c261b48a7276c79cb92a82ddfce5474bc563ba6673df30cabf17a7f239ef3", size: 39954677, archive: Archive::Zip, member: "clojure-lsp" } },
];

pub(super) static TYPESCRIPT_LANGUAGE_SERVER: &[NpmPin] = &[
    NpmPin { package: "typescript-language-server", pin: Pin { version: "6.0.3", url: "https://registry.npmjs.org/typescript-language-server/-/typescript-language-server-6.0.3.tgz", sha256: "96c1b2aa0cd72db1d653438870f3b2b4cd6b2b508ca4903ce7c798dda59bc151", size: 514069, archive: Archive::TarGz, member: "" } },
    NpmPin { package: "typescript", pin: Pin { version: "5.9.3", url: "https://registry.npmjs.org/typescript/-/typescript-5.9.3.tgz", sha256: "10e108c9cf7d5f2879053dff18515fb405abf2ccef63eaaf017d9c571687a1d3", size: 4377468, archive: Archive::TarGz, member: "" } },
];

pub(super) static PYRIGHT: &[NpmPin] = &[NpmPin {
    package: "pyright",
    pin: Pin {
        version: "1.1.414",
        url: "https://registry.npmjs.org/pyright/-/pyright-1.1.414.tgz",
        sha256: "bf5f473f6167c0d14175492c3263d783b4489a6956e1c06c18e15228e3a3fa42",
        size: 4226827,
        archive: Archive::TarGz,
        member: "",
    },
}];
