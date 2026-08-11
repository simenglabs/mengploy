//! Akses Docker di server target, HANYA lewat socket unix yang di-forward
//! lewat SSH — tidak pernah TCP (invariant 13). Fitur TCP `bollard` sengaja
//! tidak diaktifkan di `Cargo.toml` supaya jalur itu tidak terkompilasi.
//!
//! Pembagian modul:
//! - `forward` — buka/tutup local port forward socket Docker lewat sesi SSH
//!   yang sudah terbuka (`crate::ssh::SshSession`).
//! - `client` — `bollard`: ping, version, info lewat socket ter-forward.
//! - `registry_login` — `docker login` di server target lewat SSH exec
//!   (bukan lewat API `bollard`) supaya kredensial mendarat di
//!   `~/.docker/config.json` milik CLI, yang dipakai `docker run`/`docker
//!   pull` di Fase 2.

pub mod client;
pub mod forward;
pub mod registry_login;

pub use client::{
    ContainerObservation, ContainerStatsObservation, ContainerStatus, DockerClientError,
    DockerCredentials, ImageObservation, LogFollowError, NewContainer, TRAEFIK_IMAGE_TAG,
    TRAEFIK_LABEL, connect, container_exists_with_label, container_logs, container_logs_follow,
    create_container, create_traefik_container, ensure_network, inspect,
    list_containers_with_label, list_images, os_info, ping, pull_image, remove_container,
    remove_image, resolve_image_digest, start_container, stats, stop_container, version,
};
pub use forward::{DockerForward, DockerForwardError, cleanup_orphans, close, establish};
pub use registry_login::{RegistryLoginError, login};
