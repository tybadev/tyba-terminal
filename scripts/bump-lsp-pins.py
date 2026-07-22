#!/usr/bin/env python3
import hashlib
import json
import os
import subprocess
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PINS_RS = os.path.join(REPO_ROOT, "src-tauri", "src", "lsp", "managed", "pins.rs")

PLAT_ORDER = ["linux_x64", "linux_arm64", "macos_arm64", "macos_x64"]
PLAT_ENUM = {
    "linux_x64": "LinuxX86_64",
    "linux_arm64": "LinuxAarch64",
    "macos_arm64": "MacosAarch64",
    "macos_x64": "MacosX86_64",
}


def curl(url):
    cmd = ["curl", "-sSL", "--fail", "--retry", "3", "-A", "tyba-bump"]
    token = os.environ.get("GITHUB_TOKEN")
    if token and "api.github.com" in url:
        cmd += ["-H", f"Authorization: Bearer {token}"]
    cmd.append(url)
    return subprocess.run(cmd, check=True, capture_output=True).stdout


def http_json(url):
    return json.loads(curl(url))


def http_text(url):
    return curl(url).decode()


def download(url):
    return curl(url)


def latest_gh_tag(repo):
    return http_json(f"https://api.github.com/repos/{repo}/releases/latest")["tag_name"]


def latest_node_lts():
    for entry in http_json("https://nodejs.org/dist/index.json"):
        if entry["lts"]:
            return entry["version"].lstrip("v")
    raise SystemExit("nenhuma versão LTS do node encontrada")


def npm_version(pkg, keep_major=None):
    data = http_json(f"https://registry.npmjs.org/{pkg}")
    if keep_major is None:
        return data["dist-tags"]["latest"]
    candidates = [
        v
        for v in data["versions"]
        if v.startswith(f"{keep_major}.") and "-" not in v
    ]
    candidates.sort(key=lambda s: [int(x) for x in s.split(".")])
    return candidates[-1]


def sha_size(data):
    return hashlib.sha256(data).hexdigest(), len(data)


def resolve():
    ra = latest_gh_tag("rust-lang/rust-analyzer")
    taplo = latest_gh_tag("tamasfe/taplo")
    marksman = latest_gh_tag("artempyanykh/marksman")
    tfls = latest_gh_tag("hashicorp/terraform-ls").lstrip("v")
    clj = latest_gh_tag("clojure-lsp/clojure-lsp")
    node = latest_node_lts()

    urls = {}
    urls["node_linux_x64"] = f"https://nodejs.org/dist/v{node}/node-v{node}-linux-x64.tar.gz"
    urls["node_linux_arm64"] = f"https://nodejs.org/dist/v{node}/node-v{node}-linux-arm64.tar.gz"
    urls["node_macos_arm64"] = f"https://nodejs.org/dist/v{node}/node-v{node}-darwin-arm64.tar.gz"
    urls["node_macos_x64"] = f"https://nodejs.org/dist/v{node}/node-v{node}-darwin-x64.tar.gz"

    base = "https://github.com/rust-lang/rust-analyzer/releases/download"
    urls["ra_linux_x64"] = f"{base}/{ra}/rust-analyzer-x86_64-unknown-linux-gnu.gz"
    urls["ra_linux_arm64"] = f"{base}/{ra}/rust-analyzer-aarch64-unknown-linux-gnu.gz"
    urls["ra_macos_arm64"] = f"{base}/{ra}/rust-analyzer-aarch64-apple-darwin.gz"
    urls["ra_macos_x64"] = f"{base}/{ra}/rust-analyzer-x86_64-apple-darwin.gz"

    base = "https://github.com/tamasfe/taplo/releases/download"
    urls["taplo_linux_x64"] = f"{base}/{taplo}/taplo-linux-x86_64.gz"
    urls["taplo_linux_arm64"] = f"{base}/{taplo}/taplo-linux-aarch64.gz"
    urls["taplo_macos_arm64"] = f"{base}/{taplo}/taplo-darwin-aarch64.gz"
    urls["taplo_macos_x64"] = f"{base}/{taplo}/taplo-darwin-x86_64.gz"

    base = "https://github.com/artempyanykh/marksman/releases/download"
    urls["marksman_linux_x64"] = f"{base}/{marksman}/marksman-linux-x64"
    urls["marksman_linux_arm64"] = f"{base}/{marksman}/marksman-linux-arm64"
    urls["marksman_macos"] = f"{base}/{marksman}/marksman-macos"

    base = f"https://releases.hashicorp.com/terraform-ls/{tfls}"
    urls["tfls_linux_x64"] = f"{base}/terraform-ls_{tfls}_linux_amd64.zip"
    urls["tfls_linux_arm64"] = f"{base}/terraform-ls_{tfls}_linux_arm64.zip"
    urls["tfls_macos_arm64"] = f"{base}/terraform-ls_{tfls}_darwin_arm64.zip"
    urls["tfls_macos_x64"] = f"{base}/terraform-ls_{tfls}_darwin_amd64.zip"

    base = "https://github.com/clojure-lsp/clojure-lsp/releases/download"
    urls["clj_linux_x64"] = f"{base}/{clj}/clojure-lsp-native-linux-amd64.zip"
    urls["clj_linux_arm64"] = f"{base}/{clj}/clojure-lsp-native-linux-aarch64.zip"
    urls["clj_macos_arm64"] = f"{base}/{clj}/clojure-lsp-native-macos-aarch64.zip"
    urls["clj_macos_x64"] = f"{base}/{clj}/clojure-lsp-native-macos-amd64.zip"

    tsls = npm_version("typescript-language-server")
    # A companheira `typescript` fica na linha clássica 5.x: o tsserver.js que o
    # typescript-language-server dirige não vem no rewrite nativo 7.x.
    ts = npm_version("typescript", keep_major=5)
    pyright = npm_version("pyright")
    urls["npm_tsls"] = f"https://registry.npmjs.org/typescript-language-server/-/typescript-language-server-{tsls}.tgz"
    urls["npm_typescript"] = f"https://registry.npmjs.org/typescript/-/typescript-{ts}.tgz"
    urls["npm_pyright"] = f"https://registry.npmjs.org/pyright/-/pyright-{pyright}.tgz"

    versions = {
        "node": node,
        "ra": ra,
        "taplo": taplo,
        "marksman": marksman,
        "tfls": tfls,
        "clj": clj,
        "tsls": tsls,
        "typescript": ts,
        "pyright": pyright,
    }
    return urls, versions, node


def hash_all(urls, node_version):
    meta = {}
    node_shasums = http_text(
        f"https://nodejs.org/dist/v{node_version}/SHASUMS256.txt"
    )
    node_map = {}
    for line in node_shasums.splitlines():
        parts = line.split()
        if len(parts) == 2:
            node_map[parts[1]] = parts[0]
    for key, url in urls.items():
        data = download(url)
        sha, size = sha_size(data)
        if key.startswith("node_"):
            fname = url.rsplit("/", 1)[1]
            official = node_map.get(fname)
            if official and official != sha:
                raise SystemExit(f"{key}: sha256 diverge do SHASUMS256.txt oficial")
        meta[key] = (url, sha, size)
    return meta


def pin_lit(version, key, meta, archive, member):
    url, sha, size = meta[key]
    return (
        f'Pin {{ version: "{version}", url: "{url}", sha256: "{sha}", '
        f'size: {size}, archive: Archive::{archive}, member: "{member}" }}'
    )


def platform_block(name, prefix, version, meta, archive, member, macos_shared=False):
    lines = [f"pub(super) static {name}: &[PlatformPin] = &["]
    for p in PLAT_ORDER:
        key = f"{prefix}_macos" if macos_shared and p.startswith("macos") else f"{prefix}_{p}"
        lines.append(
            f"    PlatformPin {{ platform: Platform::{PLAT_ENUM[p]}, "
            f"pin: {pin_lit(version, key, meta, archive, member)} }},"
        )
    lines.append("];")
    return "\n".join(lines)


def npm_block(name, entries, meta):
    lines = [f"pub(super) static {name}: &[NpmPin] = &["]
    for pkg, key, version in entries:
        url, sha, size = meta[key]
        pin = (
            f'Pin {{ version: "{version}", url: "{url}", sha256: "{sha}", '
            f'size: {size}, archive: Archive::TarGz, member: "" }}'
        )
        lines.append(f'    NpmPin {{ package: "{pkg}", pin: {pin} }},')
    lines.append("];")
    return "\n".join(lines)


def render(meta, versions):
    out = [
        "use super::registry::{Archive, NpmPin, Pin, PlatformPin};",
        "use super::registry::Platform;",
        "",
        platform_block("NODE_RUNTIME", "node", versions["node"], meta, "TarGz", "bin/node"),
        "",
        platform_block("RUST_ANALYZER", "ra", versions["ra"], meta, "Gzip", "rust-analyzer"),
        "",
        platform_block("TAPLO", "taplo", versions["taplo"], meta, "Gzip", "taplo"),
        "",
        platform_block("MARKSMAN", "marksman", versions["marksman"], meta, "Raw", "marksman", macos_shared=True),
        "",
        platform_block("TERRAFORM_LS", "tfls", versions["tfls"], meta, "Zip", "terraform-ls"),
        "",
        platform_block("CLOJURE_LSP", "clj", versions["clj"], meta, "Zip", "clojure-lsp"),
        "",
        npm_block(
            "TYPESCRIPT_LANGUAGE_SERVER",
            [
                ("typescript-language-server", "npm_tsls", versions["tsls"]),
                ("typescript", "npm_typescript", versions["typescript"]),
            ],
            meta,
        ),
        "",
        npm_block("PYRIGHT", [("pyright", "npm_pyright", versions["pyright"])], meta),
        "",
    ]
    return "\n".join(out) + "\n"


def main():
    urls, versions, node = resolve()
    meta = hash_all(urls, node)
    content = render(meta, versions)
    with open(PINS_RS, "w") as f:
        f.write(content)
    summary = ", ".join(f"{k}={v}" for k, v in versions.items())
    sys.stdout.write(f"pins atualizados: {summary}\n")


if __name__ == "__main__":
    main()
