use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn git_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let raw = git(dir, &["rev-parse", "--git-path", name])?;
    let path = dir.join(raw);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn sanitized(value: Option<String>, allowed: fn(char) -> bool) -> String {
    value.filter(|v| v.chars().all(allowed)).unwrap_or_default()
}

fn emit_git_info(dir: &Path) {
    let commit = sanitized(git(dir, &["rev-parse", "--short", "HEAD"]), |c| {
        c.is_ascii_hexdigit()
    });
    let date = sanitized(git(dir, &["log", "-1", "--format=%cI"]), |c| {
        c.is_ascii_digit() || matches!(c, '-' | ':' | '+' | 'T' | 'Z')
    });
    println!("cargo:rustc-env=TYBA_COMMIT={commit}");
    println!("cargo:rustc-env=TYBA_COMMIT_DATE={date}");

    if let Some(head) = git_path(dir, "HEAD") {
        println!("cargo:rerun-if-changed={}", head.display());
    }
    if let Some(branch) = git(dir, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(reference) = git_path(dir, &branch) {
            println!("cargo:rerun-if-changed={}", reference.display());
        }
    }
}

fn main() {
    // Binários de teste no Windows NÃO herdam o manifesto que o `tauri_build`
    // embute no exe principal, então o comctl32 v6 (que `rfd`/`muda` importam via
    // `TaskDialogIndirect`) não é ativado e o exe de teste falha ao CARREGAR com
    // STATUS_ENTRYPOINT_NOT_FOUND — antes de qualquer teste rodar. Injeta a
    // dependência de comctl6 só nos binários de teste (não toca o app).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTDEPENDENCY:type='win32' \
             name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
             processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    emit_git_info(Path::new(&manifest));
    tauri_build::build()
}
