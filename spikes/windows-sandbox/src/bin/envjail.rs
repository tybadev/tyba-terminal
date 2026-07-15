#![cfg(windows)]

use probe::*;

fn main() {
    banner("SONDA envjail — env por allowlist: o agente não herda o ambiente do shell (princípio #6)");
    println!("Pergunta: com CreateProcessAsUserW recebendo um bloco de env MONTADO da allowlist");
    println!("(não o env herdado), o node enjaulado vê só as vars liberadas + o marcador");
    println!("TYBA_SANDBOX, e um segredo presente no env do pai NÃO vaza para dentro?\n");

    let node = match node_exe() {
        Some(p) => p,
        None => {
            println!("[ERRO] node.exe não encontrado");
            std::process::exit(2);
        }
    };

    std::env::set_var("TYBA_SECRET_LEAK", "sk-este-token-nao-pode-vazar");

    let mut allow: Vec<(String, String)> = Vec::new();
    for key in ["SystemRoot", "PATH", "TEMP", "TMP"] {
        if let Ok(v) = std::env::var(key) {
            allow.push((key.to_string(), v));
        }
    }
    allow.push(("TYBA_SANDBOX".to_string(), "windows".to_string()));
    let block = make_env_block(&allow);

    let restricted = match build_restricted(&jail_spec(true)) {
        Ok(r) => r,
        Err(e) => {
            println!("[ERRO] token da jaula: {e}");
            std::process::exit(2);
        }
    };

    let script = "const e=process.env; \
        console.log(JSON.stringify({\
        sandbox: e.TYBA_SANDBOX || null, \
        secret: e.TYBA_SECRET_LEAK || null, \
        hasPath: !!e.PATH, \
        count: Object.keys(e).length}))";
    let cmd = format!("\"{node}\" -e \"{script}\"");

    let (code, out) = match capture(restricted.token, &cmd, Some("Winsta0\\Default"), Some(&block), 0, None) {
        Ok(v) => v,
        Err(e) => {
            println!("[ERRO] spawn enjaulado: {e}");
            std::process::exit(2);
        }
    };

    let trimmed = out.trim();
    let booted = code == 0 && trimmed.contains("\"sandbox\":\"windows\"");
    let secret_absent = trimmed.contains("\"secret\":null");
    let marker_present = trimmed.contains("\"sandbox\":\"windows\"");

    println!();
    verdict(
        "controle: node inicia com o bloco de env montado (PATH+SystemRoot bastam)",
        booted,
        "prova que a allowlist não estrangulou o node — o marcador TYBA_SANDBOX chegou",
    );
    verdict(
        "marcador TYBA_SANDBOX presente dentro da jaula",
        marker_present,
        "o agente lê isso para não tentar o que sabidamente falha na jaula",
    );
    verdict(
        "segredo do env do pai (TYBA_SECRET_LEAK) NÃO vaza para a jaula",
        secret_absent,
        "env herdado carregaria tokens do shell do dono; a allowlist corta isso na origem",
    );

    println!("\nexit {code} | env visto pelo node: {trimmed}");
}
