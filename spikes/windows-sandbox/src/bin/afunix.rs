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
    for (i, b) in bytes.iter().enumerate().take(addr.sun_path.len() - 1) {
        addr.sun_path[i] = *b as i8;
    }
    (addr, std::mem::size_of::<SOCKADDR_UN>() as i32)
}

fn parent() {
    banner("SONDA C — AF_UNIX sob a jaula (transporte único cross-platform, ou não?)");
    println!("Pergunta: um filho enjaulado (IL Low) consegue conectar num socket AF_UNIX");
    println!("criado pelo pai? Se sim, o mesmo transporte do hook serve os três SOs. Se não,");
    println!("o Windows precisa do seu próprio (named pipe da sonda A).\n");

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
        if listen(listener, 1) == SOCKET_ERROR {
            println!("[ERRO] listen falhou: {}", WSAGetLastError());
            return;
        }

        let restricted = match build_restricted_low_il() {
            Ok(r) => r,
            Err(e) => {
                println!("[ERRO] montar token restrito: {e}");
                return;
            }
        };

        let exe = std::env::current_exe().unwrap();
        let cmd = format!("\"{}\" --child {}", exe.display(), path);
        let code = match spawn_with_token(restricted.token, &cmd, &[]) {
            Ok(c) => c,
            Err(e) => {
                println!("[ERRO] spawn enjaulado: {e}");
                return;
            }
        };

        let payload = if code == 0 {
            let conn = accept(listener, std::ptr::null_mut(), std::ptr::null_mut());
            let mut buf = [0u8; 16];
            let n = if conn != INVALID_SOCKET {
                recv(conn, buf.as_mut_ptr(), buf.len() as i32, 0)
            } else {
                -1
            };
            if n > 0 {
                String::from_utf8_lossy(&buf[..n as usize]).to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let _ = std::fs::remove_file(&path);

        println!();
        verdict(
            "AF_UNIX atravessa a jaula (IL Low → socket do pai)",
            payload == "ping" && code == 0,
            "se PASS, o hook pode usar UM transporte nos três SOs; se FAIL, o Windows fica com named pipe",
        );
        println!("\ncódigo de saída do filho: {code}");
        println!(
            "  0 = conectou e enviou · 2 = WSAStartup · 3 = socket · 4 = connect NEGADO · 5 = send"
        );
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
