#![cfg(windows)]

use std::process::Command;

use probe::*;

const SPIKE_USER: &str = "tyba-agent-spike";

fn main() {
    banner("SONDA layerb — Camada B (usuário dedicado + WFP): a leitura vira default-deny de verdade");
    println!("A Camada A é blacklist: o agente lê o que o dono lê, menos as deny-ACEs/rótulos IL.");
    println!("A Camada B troca isso por um USUÁRIO dedicado (tyba-agent) — o perfil do dono fica");
    println!("inacessível por não-ter-acesso, não por enumeração — e a rede vira fronteira (WFP).\n");

    if !is_elevated() {
        println!("[BLOQUEADO] esta sonda precisa de ELEVAÇÃO (UAC) — o shell atual não é admin.");
        println!("            Criar/apagar usuário local, ACLar o perfil e aplicar filtros WFP");
        println!("            são operações privilegiadas. Sem admin não há o que medir aqui.\n");
        print_plan();
        println!("\nPara rodar o spike da Camada B, abra um PowerShell COMO ADMIN e rode:");
        println!("    cargo run --manifest-path spikes\\windows-sandbox\\Cargo.toml --bin layerb");
        println!("\n(status: PENDENTE DE ELEVAÇÃO — não é falha; é a fronteira de privilégio da Camada B)");
        return;
    }

    println!("[OK] shell elevado detectado — rodando o spike do usuário dedicado.\n");
    run_dedicated_user_spike();
}

fn print_plan() {
    println!("Plano do spike da Camada B (cada peça com par positivo, como a Camada A):");
    println!("  1. USUÁRIO DEDICADO");
    println!("     - criar `tyba-agent` (NetUserAdd / New-LocalUser), senha efêmera por sessão;");
    println!("     - spawnar o agente COMO esse usuário (CreateProcessWithLogonW);");
    println!("     - MEDIR: o agente NÃO lê um arquivo do perfil do dono (negado por não-ter-acesso,");
    println!("       não por deny-ACE); CONTROLE: o dono lê normalmente.");
    println!("  2. REDE POR WFP (escopada ao SID do tyba-agent)");
    println!("     - permitir 443 outbound; NEGAR loopback e RFC1918 (corta Docker daemon, ollama,");
    println!("       e o próprio TYBA por TCP);");
    println!("     - MEDIR: o agente conecta em 443 externo (controle) e é NEGADO em 127.0.0.1 e");
    println!("       10/172.16/192.168 (a fronteira).");
    println!("  3. UNINSTALLER (parte da entrega, não um depois)");
    println!("     - remover usuário, grupo, ACLs e TODOS os filtros WFP criados;");
    println!("     - MEDIR: após uninstall, `net user tyba-agent` não existe e nenhum filtro WFP");
    println!("       nosso sobra (o Codex deixa órfãos — não repetir).");
}

fn run_dedicated_user_spike() {
    println!(">>> PRIMEIRA EXECUÇÃO ELEVADA — este caminho não foi verificado sem admin.");
    println!(">>> Faz só a peça 1 (usuário dedicado) de forma auto-limpante; WFP e spawn-as-user");
    println!(">>> completos são os próximos sub-passos, medidos aqui quando esta base passar.\n");

    let pid = std::process::id();
    let pw = format!("Tyba$pk{pid}!Aa9");

    let created = Command::new("net")
        .args(["user", SPIKE_USER, &pw, "/add"])
        .output();
    let create_ok = matches!(&created, Ok(o) if o.status.success());
    verdict(
        "cria usuário local dedicado (net user /add)",
        create_ok,
        "a base da Camada B: um principal separado cujo acesso ao perfil do dono é nulo por padrão",
    );
    if !create_ok {
        if let Ok(o) = &created {
            println!("       stderr: {}", String::from_utf8_lossy(&o.stderr).trim());
        }
        return;
    }

    let sid = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-LocalUser -Name '{SPIKE_USER}').SID.Value"),
        ])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let sid_ok = sid.starts_with("S-1-5-21");
    verdict(
        "resolve o SID do usuário dedicado",
        sid_ok,
        "o SID é o que escopa as ACLs de leitura e (adiante) os filtros WFP de rede da Camada B",
    );
    println!("       SID: {sid}");

    let removed = Command::new("net").args(["user", SPIKE_USER, "/delete"]).output();
    let remove_ok = matches!(&removed, Ok(o) if o.status.success());
    verdict(
        "uninstaller: remove o usuário dedicado (sem deixar órfão)",
        remove_ok,
        "a Camada B remove tudo que cria — o Codex deixa usuário/ACL/WFP órfãos; aqui não",
    );

    println!("\nBase da peça 1 medida. Próximo sub-passo elevado: CreateProcessWithLogonW como o");
    println!("usuário dedicado + prova de leitura negada ao perfil do dono, depois WFP por SID.");
}
