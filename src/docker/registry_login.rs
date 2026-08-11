//! `docker login` di server target, dan verifikasi/perketat izin
//! `~/.docker/config.json` (`docs/prd.md:245`, "Fase paling kritis untuk
//! Security"). Lewat SSH exec, BUKAN lewat API `bollard` — kredensial WAJIB
//! mendarat di file config CLI yang dipakai `docker run`/`docker pull` di
//! Fase 2, dan itu bukan sesuatu yang bisa dilakukan lewat HTTP API Docker.

use std::time::Duration;

use crate::ssh::{self, SshSession};

/// "`docker login` di target (menyentuh jaringan registry): 30 detik" —
/// tabel timeout `docs/plan.md`.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);
/// Perintah pendek (chmod) — sama seperti cek Docker lain di langkah 2.
const SHORT_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

/// Batas panjang detail error yang disimpan/ditampilkan — invariant 9:
/// stderr mentah tidak pernah menyelinap tak terbatas ke pemanggil (yang
/// bisa saja menyimpannya ke `servers.last_error_message`, yang punya
/// `CHECK <=500 char` di skema).
const MAX_DETAIL_CHARS: usize = 500;

#[derive(Debug)]
pub enum RegistryLoginError {
    /// `docker login` keluar dengan exit code bukan nol — kredensial
    /// ditolak registry (host salah, kredensial salah, dsb).
    Rejected {
        detail: String,
    },
    Timeout,
    Disconnected,
    Other(String),
}

/// Login ke `registry_host` sebagai `username`/`password` di server target,
/// lalu perketat izin `~/.docker/config.json` ke `0600` kalau lebih longgar
/// dari itu. Password dikirim lewat stdin (`--password-stdin`) — TIDAK
/// PERNAH sebagai argumen baris perintah, yang akan terlihat lewat `ps` di
/// server target.
pub async fn login(
    session: &SshSession,
    registry_host: &str,
    username: &str,
    password: &str,
) -> Result<(), RegistryLoginError> {
    let result = ssh::exec_with_stdin(
        session,
        "docker",
        &[
            "login",
            registry_host,
            "--username",
            username,
            "--password-stdin",
        ],
        password.as_bytes(),
        LOGIN_TIMEOUT,
    )
    .await
    .map_err(map_exec_error)?;

    if !result.success() {
        return Err(RegistryLoginError::Rejected {
            detail: truncate_detail(&result.stderr),
        });
    }

    enforce_config_permissions(session).await
}

/// Verifikasi (dan perketat kalau perlu) izin `~/.docker/config.json` di
/// server target — file itu memuat kredensial registry dalam bentuk yang
/// bisa dipakai ulang tanpa password.
async fn enforce_config_permissions(session: &SshSession) -> Result<(), RegistryLoginError> {
    let result = ssh::exec(
        session,
        "sh",
        &["-c", "chmod 600 \"$HOME/.docker/config.json\""],
        SHORT_COMMAND_TIMEOUT,
    )
    .await
    .map_err(map_exec_error)?;

    if result.success() {
        Ok(())
    } else {
        let detail = truncate_detail(&result.stderr);
        tracing::warn!(
            detail,
            "login berhasil tapi izin ~/.docker/config.json gagal diperketat"
        );
        Err(RegistryLoginError::Other(
            "login berhasil tapi izin ~/.docker/config.json tidak bisa diperketat".to_string(),
        ))
    }
}

fn truncate_detail(stderr: &str) -> String {
    stderr.chars().take(MAX_DETAIL_CHARS).collect()
}

fn map_exec_error(err: ssh::SshExecError) -> RegistryLoginError {
    match err {
        ssh::SshExecError::Timeout => RegistryLoginError::Timeout,
        ssh::SshExecError::Disconnected => RegistryLoginError::Disconnected,
        ssh::SshExecError::Other(msg) => RegistryLoginError::Other(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_detail_memotong_stderr_panjang_ke_500_karakter() {
        let stderr_panjang = "x".repeat(1000);
        let hasil = truncate_detail(&stderr_panjang);
        assert_eq!(hasil.chars().count(), MAX_DETAIL_CHARS);
    }

    #[test]
    fn truncate_detail_tidak_mengubah_stderr_pendek() {
        let hasil = truncate_detail("unauthorized: authentication required");
        assert_eq!(hasil, "unauthorized: authentication required");
    }
}
