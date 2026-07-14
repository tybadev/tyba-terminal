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
    println!(">>> Peça 1 da Camada B (usuário dedicado + isolamento de leitura), auto-limpante.");
    println!(">>> O agente roda COMO o usuário dedicado e tenta ler um segredo do perfil do dono;");
    println!(">>> par positivo: o dono lê o mesmo segredo. WFP de rede é o próximo sub-passo.\n");

    let script = format!(
        r#"$ErrorActionPreference='SilentlyContinue'
$u='{SPIKE_USER}'
$pw='Tb$k'+($PID % 10000)+'!Aa9'
$sp=ConvertTo-SecureString $pw -AsPlainText -Force
New-LocalUser -Name $u -Password $sp -AccountNeverExpires -PasswordNeverExpires | Out-Null
Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $u | Out-Null
Write-Output ("CREATED=" + [bool](Get-LocalUser -Name $u))
$sid=(Get-LocalUser -Name $u).SID.Value
Write-Output "SID=$sid"
$sec=Join-Path $env:TEMP ("tyba-sec-"+$PID)
New-Item -ItemType Directory -Path $sec -Force | Out-Null
$secret=Join-Path $sec 'secret.txt'
'SEGREDO-DO-DONO' | Set-Content -Path $secret
$sh=Join-Path 'C:\' ("tyba-shared-"+$PID)
New-Item -ItemType Directory -Path $sh -Force | Out-Null
icacls $sh /grant '*S-1-1-0:(OI)(CI)M' | Out-Null
$out=Join-Path $sh 'out.txt'
$ctrl=Get-Content -Path $secret -Raw
Write-Output ("CONTROL=" + $(if($ctrl -match 'SEGREDO'){{'ok'}}else{{'fail'}}))
$cred=New-Object System.Management.Automation.PSCredential($u,$sp)
$ar='/c type "'+$secret+'" > "'+$out+'" 2>&1'
Start-Process -FilePath 'cmd.exe' -ArgumentList $ar -Credential $cred -WorkingDirectory 'C:\' -Wait
Start-Sleep -Milliseconds 400
$res=Get-Content -Path $out -Raw
if($res -match 'SEGREDO'){{Write-Output 'AGENTREAD=leaked'}}elseif($res){{Write-Output 'AGENTREAD=denied'}}else{{Write-Output 'AGENTREAD=empty'}}
Write-Output ("RESRAW=" + ($res -replace '\r?\n',' ').Trim())
Remove-LocalUser -Name $u | Out-Null
Write-Output ("REMOVED=" + (-not [bool](Get-LocalUser -Name $u)))
Remove-Item -Recurse -Force -Path $sec,$sh"#
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .output();
    let out = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(e) => {
            println!("[ERRO] rodar o script elevado: {e}");
            return;
        }
    };

    let get = |key: &str| -> String {
        out.lines()
            .find_map(|l| l.trim().strip_prefix(&format!("{key}=")))
            .unwrap_or("")
            .trim()
            .to_string()
    };

    let created = get("CREATED").eq_ignore_ascii_case("true");
    let sid = get("SID");
    let sid_ok = sid.starts_with("S-1-5-21");
    let control_ok = get("CONTROL") == "ok";
    let agentread = get("AGENTREAD");
    let removed = get("REMOVED").eq_ignore_ascii_case("true");

    println!();
    verdict(
        "cria usuário local dedicado (New-LocalUser)",
        created,
        "a base da Camada B: um principal separado, não o dono — o isolamento é por identidade",
    );
    verdict(
        "resolve o SID do usuário dedicado",
        sid_ok,
        "o SID escopa as ACLs de leitura e (adiante) os filtros WFP de rede da Camada B",
    );
    verdict(
        "CONTROLE: o DONO lê o segredo do próprio perfil",
        control_ok,
        "prova que o segredo existe e é legível — o que for negado abaixo é o isolamento, não setup quebrado",
    );
    verdict(
        "o agente COMO usuário dedicado NÃO lê o segredo do dono",
        control_ok && agentread != "leaked" && !agentread.is_empty(),
        "leitura default-deny DE VERDADE: default-deny por não-ter-acesso (perfil do dono), não por deny-ACE enumerada",
    );
    verdict(
        "uninstaller: remove o usuário dedicado (sem deixar órfão)",
        removed,
        "a Camada B remove tudo que cria — o Codex deixa usuário/ACL/WFP órfãos; aqui não",
    );

    println!("\nSID: {sid}");
    println!("AGENTREAD={agentread} | RESRAW: {:?}", get("RESRAW"));
    if agentread == "empty" {
        println!("(out.txt vazio → o Start-Process como usuário pode ter falhado no logon; ver RESRAW)");
    }
    println!("\nPeça 1 (isolamento de leitura por usuário dedicado) medida. Próximo sub-passo:");
    println!("WFP por SID — permitir 443, negar loopback/RFC1918 — com par positivo.");
}
