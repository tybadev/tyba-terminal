#![cfg(windows)]

use probe::*;
use windows_sys::Win32::Networking::WinSock::*;

const AF_UNIX_FAMILY: u16 = 1;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--child" {
        std::process::exit(child(&args[2]));
    }
    parent();
}

fn sockaddr_un(path: &str) -> (SOCKADDR_UN, i32) {
    let mut addr: SOCKADDR_UN = unsafe { std::mem::zeroed() };
    addr.sun_family = AF_UNIX_FAMILY;
    let bytes = path.as_bytes();
    let n = bytes.len().min(addr.sun_path.len() - 1);
    for (i, b) in bytes.iter().enumerate().take(n) {
        addr.sun_path[i] = *b as i8;
    }
    // addrlen = offset de sun_path (sun_family: u16 = 2 bytes) + path + NUL
    let len = (2 + n + 1) as i32;
    (addr, len)
}

fn accept_ping(listener: SOCKET) -> String {
    unsafe {
        let conn = accept(listener, std::ptr::null_mut(), std::ptr::null_mut());
        if conn == INVALID_SOCKET {
            return String::new();
        }
        let mut buf = [0u8; 16];
        let n = recv(conn, buf.as_mut_ptr(), buf.len() as i32, 0);
        closesocket(conn);
        if n > 0 {
            String::from_utf8_lossy(&buf[..n as usize]).to_string()
        } else {
            String::new()
        }
    }
}

fn parent() {
    banner("SONDA C — AF_UNIX sob a jaula (transporte único cross-platform, ou não?)");
    println!("Pergunta: um filho enjaulado (IL Low) consegue conectar num socket AF_UNIX");
    println!("criado pelo pai? PAR POSITIVO: um filho NÃO-enjaulado conecta no mesmo socket —");
    println!("sem isso, um 'connect falhou' não distingue jaula de AF_UNIX simplesmente não");
    println!("funcionar aqui. Só a diferença entre os dois atribui a negação à jaula.\n");

    unsafe {
        let mut wsa: WSADATA = std::mem::zeroed();
        if WSAStartup(0x0202, &mut wsa) != 0 {
            println!("[ERRO] WSAStartup falhou: {}", WSAGetLastError());
            return;
        }

        let dir = std::env::temp_dir();
        let path = dir
            .join(format!("tyba-spike-{}.sock", std::process::id()))
            .to_string_lossy()
            .to_string();
        let _ = std::fs::remove_file(&path);

        let listener = socket(AF_UNIX as i32, SOCK_STREAM as i32, 0);
        if listener == INVALID_SOCKET {
            println!("[ERRO] socket() falhou: {}", WSAGetLastError());
            return;
        }
        let (addr, len) = sockaddr_un(&path);
        if bind(listener, &addr as *const _ as *const SOCKADDR, len) == SOCKET_ERROR {
            println!("[ERRO] bind({path}) falhou: {}", WSAGetLastError());
            return;
        }
        if listen(listener, 2) == SOCKET_ERROR {
            println!("[ERRO] listen falhou: {}", WSAGetLastError());
            return;
        }

        let exe = std::env::current_exe().unwrap();

        // PAR POSITIVO: filho NÃO-enjaulado conecta no socket.
        let mut control = std::process::Command::new(&exe)
            .arg("--child")
            .arg(&path)
            .spawn()
            .expect("spawn controle");
        let control_payload = accept_ping(listener);
        let control_code = control.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        let control_ok = control_code == 0 && control_payload == "ping";

        // TESTE ENJAULADO: mesmo socket, filho sob a jaula (IL Low).
        let restricted = match build_restricted(&jail_spec(true)) {
            Ok(r) => r,
            Err(e) => {
                println!("[ERRO] montar token restrito: {e}");
                return;
            }
        };
        let cmd = format!("\"{}\" --child {}", exe.display(), path);
        let jailed_code = match spawn_with_token(restricted.token, &cmd, &[]) {
            Ok(c) => c,
            Err(e) => {
                println!("[ERRO] spawn enjaulado: {e}");
                return;
            }
        };
        let jailed_payload = if jailed_code == 0 {
            accept_ping(listener)
        } else {
            String::new()
        };
        let jailed_connected = jailed_code == 0 && jailed_payload == "ping";

        let _ = std::fs::remove_file(&path);

        println!();
        verdict(
            "CONTROLE: processo NÃO-enjaulado conecta no socket AF_UNIX",
            control_ok,
            "prova que o socket é conectável — se isto falhar, AF_UNIX não serve aqui e nada abaixo é atribuível à jaula",
        );
        verdict(
            "a jaula BLOQUEIA AF_UNIX (filho enjaulado: connect negado)",
            control_ok && !jailed_connected,
            "com o controle conectando e o enjaulado não, a negação é da jaula (IL Low → socket do pai). Windows fica com named pipe",
        );

        println!("\ncontrole  -> exit {control_code} | payload {control_payload:?}");
        println!("enjaulado -> exit {jailed_code} | payload {jailed_payload:?}");
        println!("  (filho: 0=conectou+enviou · 2=WSAStartup · 3=socket · 4=connect NEGADO · 5=send)");
    }
}

fn child(path: &str) -> i32 {
    unsafe {
        let mut wsa: WSADATA = std::mem::zeroed();
        if WSAStartup(0x0202, &mut wsa) != 0 {
            return 2;
        }
        let s = socket(AF_UNIX as i32, SOCK_STREAM as i32, 0);
        if s == INVALID_SOCKET {
            return 3;
        }
        let (addr, len) = sockaddr_un(path);
        if connect(s, &addr as *const _ as *const SOCKADDR, len) == SOCKET_ERROR {
            return 4;
        }
        let sent = send(s, b"ping".as_ptr(), 4, 0);
        closesocket(s);
        if sent != 4 {
            return 5;
        }
        0
    }
}
