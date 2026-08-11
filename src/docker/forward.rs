//! Forward socket Docker (`/var/run/docker.sock`) di server target ke
//! socket unix lokal lewat SSH local port forward. Satu-satunya jalur
//! menjangkau Docker — invariant 13 (tidak pernah TCP) berlaku karena
//! fitur TCP `bollard` tidak diaktifkan di `Cargo.toml`, jadi jalur
//! `DOCKER_HOST=tcp://` tidak ada untuk dipanggil sama sekali.
//!
//! **Catatan jujur**: `establish` mengasumsikan file socket lokal sudah
//! ada tepat saat `request_port_forward` selesai (dipakai untuk chmod
//! `0600` segera). Ini belum pernah diverifikasi terhadap server SSH nyata
//! di lingkungan ini (tidak ada Docker-in-Docker di sandbox pengembangan) —
//! kalau asumsi ini salah, `set_mode` gagal dengan `NotFound` dan
//! `establish` mengembalikan `DockerForwardError::Other`, bukan panik.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ssh::SshSession;

/// Path socket Docker standar di server target. Fase 1 tidak mendukung
/// path kustom — cukup untuk instalasi Docker default (`docs/plan.md`
/// tidak meminta lebih).
const REMOTE_DOCKER_SOCKET: &str = "/var/run/docker.sock";

/// "Siapkan forward socket Docker: 10 detik" — tabel timeout `docs/plan.md`.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(10);

/// Forward socket Docker aktif untuk satu server. SSH port-forward hanya
/// bisa ditutup lewat panggilan `async`, jadi TIDAK ADA `Drop` di sini —
/// pemanggil wajib memanggil [`close`] eksplisit di semua jalur keluar
/// (verifikasi selesai, shutdown aplikasi). Lihat risiko di `docs/plan.md`.
pub struct DockerForward {
    listen_socket: PathBuf,
}

#[derive(Debug)]
pub enum DockerForwardError {
    Timeout,
    Other(String),
}

impl DockerForward {
    pub fn socket_path(&self) -> &Path {
        &self.listen_socket
    }
}

/// Buka forward socket Docker milik `server_id` di dalam `runtime_dir`
/// (mis. `{data_dir}/runtime`). Satu socket per server, mode `0600`,
/// direktori induk `0700`.
pub async fn establish(
    session: &SshSession,
    runtime_dir: &Path,
    server_id: &str,
) -> Result<DockerForward, DockerForwardError> {
    let dir = runtime_dir.join("docker-sock");
    std::fs::create_dir_all(&dir).map_err(other)?;
    set_mode(&dir, 0o700).map_err(other)?;

    let listen_socket = dir.join(format!("{server_id}.sock"));
    // Sisa socket dari proses sebelumnya (mis. crash, bukan shutdown
    // bersih) dihapus dulu — `ssh` menolak bind kalau file sudah ada.
    remove_if_exists(&listen_socket).map_err(other)?;

    let remote = Path::new(REMOTE_DOCKER_SOCKET);

    let hasil = tokio::time::timeout(
        FORWARD_TIMEOUT,
        session.forward_unix_local(&listen_socket, remote),
    )
    .await;
    match hasil {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "gagal membuka forward socket docker");
            let _ = session
                .close_unix_local_forward(&listen_socket, remote)
                .await;
            let _ = remove_if_exists(&listen_socket);
            return Err(DockerForwardError::Other(
                "gagal membuka forward socket docker".to_string(),
            ));
        }
        Err(_) => {
            // Request forward yang timeout bisa sudah membuat bind lokal;
            // tutup best-effort sebelum mengembalikan error agar tidak ada
            // socket/forward parsial yang menunggu cleanup startup.
            let _ = session
                .close_unix_local_forward(&listen_socket, remote)
                .await;
            let _ = remove_if_exists(&listen_socket);
            return Err(DockerForwardError::Timeout);
        }
    }

    if let Err(err) = set_mode(&listen_socket, 0o600) {
        let _ = session
            .close_unix_local_forward(&listen_socket, remote)
            .await;
        let _ = remove_if_exists(&listen_socket);
        return Err(other(err));
    }

    Ok(DockerForward { listen_socket })
}

/// Tutup forward yang dibuka [`establish`] dan hapus socket lokal. Dipanggil
/// di semua jalur keluar verifikasi (sukses maupun gagal) dan saat shutdown
/// aplikasi — socket forward tidak boleh menumpuk (invariant izin file,
/// `docs/plan.md` risiko).
pub async fn close(session: &SshSession, forward: DockerForward) {
    let remote = Path::new(REMOTE_DOCKER_SOCKET);
    if let Err(err) = session
        .close_unix_local_forward(&forward.listen_socket, remote)
        .await
    {
        tracing::warn!(error = %err, "gagal menutup forward socket docker di sisi ssh (dilanjutkan)");
    }
    if let Err(err) = remove_if_exists(&forward.listen_socket) {
        tracing::warn!(error = %err, "gagal menghapus file socket forward lokal");
    }
}

/// Hapus socket yatim dari proses sebelumnya. Dipanggil `main.rs` saat
/// startup — forward yang tersisa setelah crash tidak pernah dianggap
/// tepercaya, selalu dibuang (`docs/plan.md`: "Socket yatim dari proses
/// sebelumnya dibersihkan saat startup").
pub fn cleanup_orphans(runtime_dir: &Path) {
    let dir = runtime_dir.join("docker-sock");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        if let Err(err) = std::fs::remove_file(entry.path()) {
            tracing::warn!(
                error = %err,
                path = %entry.path().display(),
                "gagal menghapus socket forward yatim"
            );
        }
    }
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    let metadata = std::fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}

fn other(err: io::Error) -> DockerForwardError {
    DockerForwardError::Other(format!("gagal menyiapkan socket forward lokal: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_orphans_menghapus_semua_file_di_direktori_docker_sock() {
        let base = std::env::temp_dir().join(format!(
            "mengdep-test-docker-forward-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let sock_dir = base.join("docker-sock");
        std::fs::create_dir_all(&sock_dir).unwrap();
        std::fs::write(sock_dir.join("server-a.sock"), b"").unwrap();
        std::fs::write(sock_dir.join("server-b.sock"), b"").unwrap();

        cleanup_orphans(&base);

        let sisa: Vec<_> = std::fs::read_dir(&sock_dir)
            .map(|entries| entries.filter_map(Result::ok).collect())
            .unwrap_or_default();
        assert!(sisa.is_empty(), "semua socket yatim harus terhapus");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_orphans_tidak_panik_kalau_direktori_belum_ada() {
        let base = std::env::temp_dir().join(format!(
            "mengdep-test-docker-forward-belum-ada-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));

        cleanup_orphans(&base); // tidak boleh panik walau direktori tidak ada
    }
}
