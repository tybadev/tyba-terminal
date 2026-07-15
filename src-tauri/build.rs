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
    tauri_build::build()
}
