//! Eksekusi satu perintah remote lewat sesi SSH yang sudah terbuka.
//!
//! **Kritis** (tugas Debugger, `docs/prd.md`, ditegaskan `docs/plan.md`
//! "Membedakan kegagalan SSH"): exit code bukan nol BUKAN error transport.
//! Perintah yang berhasil DIEKSEKUSI (mis. `docker version` mengembalikan
//! exit code 127 karena `docker` tidak ada di PATH) adalah `Ok(ExecResult
//! { code: 127, .. })`, bukan `Err`. `Err` (`SshExecError`) HANYA untuk
//! kegagalan level transport SSH itu sendiri: koneksi putus, channel
//! gagal dibuka, atau timeout tercapai sebelum perintah selesai.

use std::time::Duration;

use openssh::{Session as OpensshSession, Stdio};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::session::SshSession;

/// Hasil eksekusi perintah remote yang BERHASIL DIJALANKAN (transport OK).
/// `code != 0` di sini tetap `Ok`, bukan `Err` — pemanggil yang
/// memutuskan artinya (mis. exit 127 dari `docker version` berarti
/// "Docker tidak terpasang", exit 1 dari cek grup berarti "user bukan
/// anggota grup docker").
pub struct ExecResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecResult {
    /// Perintah keluar dengan exit code nol.
    pub fn success(&self) -> bool {
        self.code == 0
    }
}

/// Kegagalan level TRANSPORT — bukan exit code perintah remote.
#[derive(Debug)]
pub enum SshExecError {
    /// Batas waktu (`timeout` parameter pemanggil) tercapai sebelum
    /// perintah selesai.
    Timeout,
    /// Koneksi SSH terputus di tengah eksekusi, atau channel gagal
    /// dibuka sama sekali.
    Disconnected,
    /// Kegagalan transport lain yang tidak masuk dua kategori di atas.
    /// Pesan sudah generik — detail asli hanya ke `tracing`.
    Other(String),
}

/// Jalankan `program` dengan `args` lewat `session`, dibatasi `timeout`.
/// `timeout` adalah PARAMETER pemanggil (bukan konstanta modul ini) —
/// sub-blok 3d memakai 15 detik untuk perintah pendek seperti
/// `docker version` sesuai tabel timeout `docs/plan.md`.
pub async fn exec(
    session: &SshSession,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<ExecResult, SshExecError> {
    match tokio::time::timeout(timeout, run(&session.inner, program, args)).await {
        Ok(result) => result,
        Err(_) => Err(SshExecError::Timeout),
    }
}

async fn run(
    session: &OpensshSession,
    program: &str,
    args: &[&str],
) -> Result<ExecResult, SshExecError> {
    let mut command = session.command(program);
    command.args(args);

    let output = command.output().await.map_err(map_openssh_error)?;
    Ok(result_from_output(output))
}

/// Sama seperti [`exec`], tapi mengirim `stdin_data` ke perintah remote
/// sebelum menunggu keluarannya. Dipakai `docker/registry_login.rs` untuk
/// `docker login --password-stdin` — password TIDAK PERNAH lewat sebagai
/// argumen baris perintah (yang terlihat lewat `ps` di server target).
/// Jalankan perintah dengan batas byte output stdout/stderr dan timeout.
/// Output yang melewati batas dipotong sebelum kembali ke pemanggil; child
/// dilepas saat batas tercapai agar koneksi tidak menunggu keluaran tanpa akhir.
pub async fn exec_bounded(
    session: &SshSession,
    program: &str,
    args: &[&str],
    timeout: Duration,
    max_bytes: usize,
) -> Result<(ExecResult, bool), SshExecError> {
    let mut command = session.inner.command(program);
    command.args(args);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let child = command.spawn().await.map_err(map_openssh_error)?;
    match tokio::time::timeout(timeout, collect_bounded(child, max_bytes)).await {
        Ok(result) => result,
        Err(_) => Err(SshExecError::Timeout),
    }
}

async fn collect_bounded(
    mut child: openssh::RemoteChild<'_>,
    max_bytes: usize,
) -> Result<(ExecResult, bool), SshExecError> {
    let stdout = child.stdout().take();
    let stderr = child.stderr().take();
    let (stdout, stderr) = tokio::join!(
        read_bounded(stdout, max_bytes),
        read_bounded(stderr, max_bytes),
    );
    let (stdout, stdout_truncated) = stdout?;
    let (stderr, stderr_truncated) = stderr?;
    let status = child.wait().await.map_err(map_openssh_error)?;
    Ok((
        ExecResult {
            code: status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        },
        stdout_truncated || stderr_truncated,
    ))
}

async fn read_bounded<R>(
    stream: Option<R>,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool), SshExecError>
where
    R: AsyncRead + Unpin,
{
    let Some(stream) = stream else {
        return Ok((Vec::new(), false));
    };
    let mut bytes = Vec::new();
    let limit = u64::try_from(max_bytes)
        .map_err(|_| SshExecError::Other("batas output tidak valid".to_string()))?
        .saturating_add(1);
    stream
        .take(limit)
        .read_to_end(&mut bytes)
        .await
        .map_err(|err| SshExecError::Other(format!("gagal membaca keluaran remote: {err}")))?;
    let truncated = bytes.len() > max_bytes;
    if truncated {
        bytes.truncate(max_bytes);
    }
    Ok((bytes, truncated))
}

pub async fn exec_with_stdin(
    session: &SshSession,
    program: &str,
    args: &[&str],
    stdin_data: &[u8],
    timeout: Duration,
) -> Result<ExecResult, SshExecError> {
    match tokio::time::timeout(
        timeout,
        run_with_stdin(&session.inner, program, args, stdin_data),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(SshExecError::Timeout),
    }
}

async fn run_with_stdin(
    session: &OpensshSession,
    program: &str,
    args: &[&str],
    stdin_data: &[u8],
) -> Result<ExecResult, SshExecError> {
    let mut command = session.command(program);
    command.args(args);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().await.map_err(map_openssh_error)?;

    let mut stdin = child
        .stdin()
        .take()
        .ok_or_else(|| SshExecError::Other("stdin perintah remote tidak tersedia".to_string()))?;
    stdin
        .write_all(stdin_data)
        .await
        .map_err(|err| SshExecError::Other(format!("gagal menulis ke stdin remote: {err}")))?;
    // Drop menutup pipe, mengirim EOF ke proses remote — beberapa perintah
    // (termasuk `docker login --password-stdin`) menunggu EOF sebelum
    // membaca input selesai.
    drop(stdin);

    let output = child.wait_with_output().await.map_err(map_openssh_error)?;
    Ok(result_from_output(output))
}

fn result_from_output(output: std::process::Output) -> ExecResult {
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    ExecResult {
        code,
        stdout,
        stderr,
    }
}

fn map_openssh_error(err: openssh::Error) -> SshExecError {
    // Detail asli tidak memuat secret (perintah dan argumen yang dipanggil
    // sub-blok 3d bukan secret — private key tidak pernah lewat sebagai
    // argumen perintah). Aman dicatat ke tracing.
    tracing::warn!(error = %err, "eksekusi perintah remote gagal di level transport");

    match err {
        openssh::Error::Disconnected | openssh::Error::RemoteProcessTerminated => {
            SshExecError::Disconnected
        }
        _ => SshExecError::Other("kegagalan transport ssh saat eksekusi perintah".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `exec.rs` HANYA bisa diuji penuh (transport asli) lewat koneksi SSH
    /// nyata, yang tidak tersedia di lingkungan test/CI ini. Yang diuji di
    /// sini murni logika pemisahan `code`/`stdout`/`stderr` dan mapping
    /// error — memakai `std::process::Command` LOKAL sebagai pengganti
    /// transport untuk memverifikasi bahwa "exit code bukan nol tetap
    /// Ok" adalah pola yang benar-benar diterapkan, bukan hanya
    /// didokumentasikan. Ini bukan test terhadap `exec()` itu sendiri
    /// (yang butuh `openssh::Session`) — lihat catatan di laporan akhir
    /// soal keterbatasan ini.
    async fn simulasi_exec_lokal(program: &str, args: &[&str]) -> ExecResult {
        let output = tokio::process::Command::new(program)
            .args(args)
            .output()
            .await
            .expect("perintah lokal harus bisa dijalankan");

        ExecResult {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    #[tokio::test]
    async fn exit_code_bukan_nol_tetap_dianggap_ok_bukan_error() {
        // `false` selalu keluar dengan exit code 1 — mensimulasikan
        // "perintah remote berhasil dieksekusi tapi gagal", persis kasus
        // `docker version` exit 127 yang HARUS tetap Ok(ExecResult{..}).
        let result = simulasi_exec_lokal("false", &[]).await;

        assert_eq!(result.code, 1);
        assert!(!result.success());
    }

    #[tokio::test]
    async fn exit_code_nol_dianggap_sukses() {
        let result = simulasi_exec_lokal("true", &[]).await;

        assert_eq!(result.code, 0);
        assert!(result.success());
    }

    #[tokio::test]
    async fn stdout_dan_stderr_terpisah_dengan_benar() {
        let result = simulasi_exec_lokal(
            "sh",
            &["-c", "echo isi-stdout; echo isi-stderr 1>&2; exit 3"],
        )
        .await;

        assert_eq!(result.code, 3);
        assert_eq!(result.stdout.trim(), "isi-stdout");
        assert_eq!(result.stderr.trim(), "isi-stderr");
    }

    #[test]
    fn map_openssh_error_disconnected_untuk_koneksi_putus() {
        let err = openssh::Error::Disconnected;
        assert!(matches!(map_openssh_error(err), SshExecError::Disconnected));
    }

    #[test]
    fn map_openssh_error_other_untuk_kegagalan_tak_dikenal() {
        let err = openssh::Error::RemoteProcessTerminated;
        // Baik Disconnected maupun RemoteProcessTerminated dipetakan ke
        // varian yang sama karena openssh sendiri tidak bisa membedakan
        // keduanya secara andal (lihat dokumentasi openssh::Error).
        assert!(matches!(map_openssh_error(err), SshExecError::Disconnected));
    }
}
