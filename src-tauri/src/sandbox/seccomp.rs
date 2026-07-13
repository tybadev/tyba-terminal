use std::collections::BTreeMap;

use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, SeccompRule, TargetArch};

pub const DENIED_SYSCALLS: [i64; 12] = [
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
    libc::SYS_open_by_handle_at,
    libc::SYS_kexec_load,
    libc::SYS_kexec_file_load,
    libc::SYS_keyctl,
    libc::SYS_add_key,
    libc::SYS_request_key,
];

#[cfg(target_arch = "x86_64")]
const ARCH: TargetArch = TargetArch::x86_64;
#[cfg(target_arch = "aarch64")]
const ARCH: TargetArch = TargetArch::aarch64;

pub fn compile() -> Result<Vec<u8>, String> {
    let rules: BTreeMap<i64, Vec<SeccompRule>> =
        DENIED_SYSCALLS.iter().map(|s| (*s, Vec::new())).collect();
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        ARCH,
    )
    .map_err(|e| format!("filtro seccomp: {e}"))?;
    let program: BpfProgram = filter
        .try_into()
        .map_err(|e: seccompiler::BackendError| format!("compilação do BPF: {e}"))?;
    let mut bytes = Vec::with_capacity(program.len() * 8);
    for insn in &program {
        bytes.extend_from_slice(&insn.code.to_ne_bytes());
        bytes.push(insn.jt);
        bytes.push(insn.jf);
        bytes.extend_from_slice(&insn.k.to_ne_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_to_nonempty_aligned_program() {
        let bytes = compile().unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(bytes.len() % 8, 0, "sock_filter tem 8 bytes");
        assert!(
            bytes.len() / 8 > DENIED_SYSCALLS.len(),
            "programa precisa de ao menos uma instrução por syscall negada"
        );
    }

    #[test]
    fn denylist_never_touches_recvfrom() {
        assert!(
            !DENIED_SYSCALLS.contains(&libc::SYS_recvfrom),
            "negar recvfrom quebra cargo clippy (achado nº 9 do source do Codex)"
        );
    }
}
