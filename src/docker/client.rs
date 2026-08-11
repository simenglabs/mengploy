//! Klien Docker lewat `bollard`, menjangkau socket HANYA lewat unix socket
//! forward lokal (`docker/forward.rs`) — tidak pernah TCP (invariant 13).
//! Fitur TCP `bollard` sengaja tidak diaktifkan di `Cargo.toml`, jadi
//! `Docker::connect_with_http`/`connect_with_ssl` tidak ada untuk dipanggil.
//!
//! Fase 2 menambah operasi container lifecycle (pull/create/start/inspect/
//! logs/stop/remove) — dipakai `deployments::engine`, BUKAN dipanggil
//! langsung dari `routes/**` (batas modul sama seperti `servers::verify`).

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use bollard::API_DEFAULT_VERSION;
use bollard::Docker;
pub use bollard::auth::DockerCredentials;
use bollard::models::{
    ContainerCreateBody, ContainerSummaryStateEnum, HostConfig, NetworkCreateRequest, PortBinding,
    RestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptionsBuilder,
    ListImagesOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveImageOptionsBuilder, StatsOptionsBuilder, StopContainerOptionsBuilder,
};
use tokio_stream::StreamExt as _;

/// "bollard ping lewat socket ter-forward: 5 detik" — tabel timeout
/// `docs/plan.md`. Dipakai untuk setiap panggilan tunggal SINGKAT di modul
/// ini (`ping`, `version`, `os_info`, `create_container`, `start_container`,
/// `inspect`, `stop_container`, `remove_container`), bukan satu timeout
/// gabungan. `pull_image` dan `container_logs` punya batasnya sendiri
/// (operasi yang secara inheren bisa berlangsung lama).
const CLIENT_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// "Start container: 30 detik" — tabel timeout `docs/plan.md`
/// (`docs/prd.md` §9 versi CLAUDE.md lama, dikutip ulang `docs/plan.md`
/// Fase 2).
const START_CONTAINER_TIMEOUT: Duration = Duration::from_secs(30);

/// "Drain container lama: 30 detik" — dipakai juga sebagai timeout REQUEST
/// `stop_container` di sini; nilai `t` (grace period docker) dioper terpisah
/// oleh pemanggil.
const STOP_CONTAINER_TIMEOUT: Duration = Duration::from_secs(35);
/// Satu penghapusan image tidak boleh menahan lease server tanpa batas.
const REMOVE_IMAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// "Pull image: 10 menit total, ATAU 60 detik tanpa progres byte" —
/// `docs/plan.md` Fase 2.
const PULL_STALL_TIMEOUT: Duration = Duration::from_secs(60);
const PULL_TOTAL_TIMEOUT: Duration = Duration::from_secs(600);

/// Ambil log dalam batas wajar — dipakai untuk menangkap 50 baris terakhir
/// container yang gagal (invariant §3 no.7), bukan streaming log runtime
/// (itu Fase 3).
const LOGS_TIMEOUT: Duration = Duration::from_secs(10);

/// "Chunk pertama dari `docker logs --follow`: 15 detik" — tabel timeout
/// `docs/plan.md` Fase 3. Batas ini HANYA membungkus chunk pertama; setelah
/// itu sunyi BUKAN error dan sisa stream tidak dibungkus timeout apa pun
/// (invariant §3 no.11: timeout per tahap, bukan timeout global). Batas 30
/// menit per sesi diatur pemanggil (`routes/events.rs`), bukan di sini.
const LOGS_FOLLOW_FIRST_CHUNK_TIMEOUT: Duration = Duration::from_secs(15);

/// Bootstrap Traefik lazy (`docs/plan.md` Q2, CLAUDE.md §4 "Traefik, Docker
/// label discovery"). Label penanda, nama container tetap, dan tag pull —
/// SEMUA literal, satu Traefik per server, tidak configurable.
pub const TRAEFIK_LABEL: &str = "platform.traefik=true";
const TRAEFIK_CONTAINER_NAME: &str = "platform-traefik";
pub const TRAEFIK_IMAGE_TAG: &str = "traefik:v3.1";

#[derive(Debug)]
pub enum DockerClientError {
    /// Socket tidak ada, koneksi ditolak, atau panggilan API gagal.
    Unreachable,
    Timeout,
    Other(String),
}

/// Sambungkan ke Docker lewat socket unix lokal `socket_path` (hasil
/// `docker/forward.rs::establish`). Operasi ini murah dan sinkron —
/// `bollard` hanya memeriksa file socket ada, belum benar-benar
/// berkomunikasi dengan daemon (itu baru terjadi di [`ping`]).
pub fn connect(socket_path: &Path) -> Result<Docker, DockerClientError> {
    let path = socket_path.to_string_lossy();
    Docker::connect_with_unix(&path, CLIENT_CALL_TIMEOUT.as_secs(), API_DEFAULT_VERSION).map_err(
        |err| {
            tracing::warn!(error = %err, "gagal menyambung ke docker lewat socket forward lokal");
            DockerClientError::Unreachable
        },
    )
}

/// Sub-cek (d) `docs/plan.md` "Langkah 2 — Docker": forward socket + bollard
/// ping berhasil.
pub async fn ping(docker: &Docker) -> Result<(), DockerClientError> {
    call(docker.ping()).await.map(|_| ())
}

/// Versi engine Docker untuk `servers.docker_version`.
pub async fn version(docker: &Docker) -> Result<String, DockerClientError> {
    let info = call(docker.version()).await?;
    Ok(info
        .version
        .unwrap_or_else(|| "tidak diketahui".to_string()))
}

/// Ringkasan OS server target untuk `servers.os_info`, digabung dari
/// `operating_system`, `architecture`, dan `kernel_version` — field yang
/// hilang dilewati, bukan diisi placeholder kosong.
pub async fn os_info(docker: &Docker) -> Result<String, DockerClientError> {
    let info = call(docker.info()).await?;
    let parts: Vec<String> = [
        info.operating_system,
        info.architecture,
        info.kernel_version.map(|kernel| format!("kernel {kernel}")),
    ]
    .into_iter()
    .flatten()
    .collect();

    if parts.is_empty() {
        Ok("tidak diketahui".to_string())
    } else {
        Ok(parts.join(", "))
    }
}

async fn call<T, F>(fut: F) -> Result<T, DockerClientError>
where
    F: std::future::Future<Output = Result<T, bollard::errors::Error>>,
{
    call_dengan_timeout(CLIENT_CALL_TIMEOUT, fut).await
}

async fn call_dengan_timeout<T, F>(timeout: Duration, fut: F) -> Result<T, DockerClientError>
where
    F: std::future::Future<Output = Result<T, bollard::errors::Error>>,
{
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "panggilan docker gagal");
            Err(DockerClientError::Unreachable)
        }
        Err(_) => Err(DockerClientError::Timeout),
    }
}

/// Tarik image lewat referensi digest LENGKAP (mis.
/// `ghcr.io/org/app@sha256:...`). `credentials` opsional — `None` untuk
/// image publik; pemanggil (`deployments::engine`) yang mencocokkan host
/// image ke registry tersimpan dan mendekripsi tokennya, modul ini tidak
/// tahu apa pun soal db.
///
/// Berhenti lebih awal dengan `Timeout` kalau tidak ada progres byte
/// selama `PULL_STALL_TIMEOUT`, ATAU total operasi melebihi
/// `PULL_TOTAL_TIMEOUT` — dua batas terpisah, bukan satu timeout global
/// (invariant 11).
pub async fn pull_image(
    docker: &Docker,
    image_ref: &str,
    credentials: Option<DockerCredentials>,
) -> Result<(), DockerClientError> {
    let options = CreateImageOptionsBuilder::default()
        .from_image(image_ref)
        .build();
    let mut stream = docker.create_image(Some(options), None, credentials);

    let hasil = tokio::time::timeout(PULL_TOTAL_TIMEOUT, async {
        loop {
            match tokio::time::timeout(PULL_STALL_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(_progres))) => {} // progres diterima, lanjut — stall timer otomatis reset lewat loop
                Ok(Some(Err(err))) => {
                    tracing::warn!(error = %err, "pull image gagal");
                    return Err(DockerClientError::Other(err.to_string()));
                }
                Ok(None) => return Ok(()), // stream selesai = pull sukses
                Err(_) => return Err(DockerClientError::Timeout), // macet tanpa progres
            }
        }
    })
    .await;

    match hasil {
        Ok(inner) => inner,
        Err(_) => Err(DockerClientError::Timeout), // total pull kelamaan
    }
}

/// Spesifikasi container baru — parameter generik docker, TIDAK tahu apa
/// pun soal `apps`/`deployments` (tipe domain itu tetap di
/// `deployments::engine`, yang membangun `labels` dari baris db).
pub struct NewContainer<'a> {
    pub name: &'a str,
    pub image_ref: &'a str,
    pub network: &'a str,
    pub labels: &'a [(String, String)],
    /// `(KEY, value)` — diteruskan ke `ContainerCreateBody.env` (field API
    /// `bollard`, bukan argumen shell `-e`; `docs/plan.md` Fase 4 "Q1
    /// opsi A"). `docker inspect` di server target akan menampilkan
    /// nilai ini apa adanya — batasan yang melekat pada Docker Engine API
    /// itu sendiri, bukan sesuatu yang bisa dihindari lewat mekanisme
    /// pengiriman yang berbeda (lihat `docs/plan.md` "Pertanyaan
    /// terbuka" Q1 untuk penjelasan lengkap).
    pub env: &'a [(String, String)],
}

/// Buat container (belum jalan — `start_container` terpisah, supaya
/// kegagalan create vs start bisa dibedakan pemanggil). `--restart
/// unless-stopped` dan `--network {network}` selalu diset, TIDAK PERNAH
/// `-p` (invariant §5 no.5, dan dua container lama+baru harus bisa hidup
/// bersamaan tanpa tabrakan port — `docs/plan.md`).
pub async fn create_container(
    docker: &Docker,
    spec: NewContainer<'_>,
) -> Result<String, DockerClientError> {
    let options = CreateContainerOptionsBuilder::default()
        .name(spec.name)
        .build();

    let labels: HashMap<String, String> = spec.labels.iter().cloned().collect();
    let env: Option<Vec<String>> = if spec.env.is_empty() {
        None
    } else {
        Some(spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect())
    };

    let body = ContainerCreateBody {
        image: Some(spec.image_ref.to_string()),
        labels: Some(labels),
        env,
        host_config: Some(HostConfig {
            network_mode: Some(spec.network.to_string()),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let response = call(docker.create_container(Some(options), body)).await?;
    Ok(response.id)
}

pub async fn start_container(docker: &Docker, container_id: &str) -> Result<(), DockerClientError> {
    call_dengan_timeout(
        START_CONTAINER_TIMEOUT,
        docker.start_container(container_id, None),
    )
    .await
}

/// Status ringkas container — cukup untuk klasifikasi kegagalan health
/// check (`deployments::engine`), bukan `ContainerInspectResponse` mentah.
pub struct ContainerStatus {
    pub running: bool,
    pub exit_code: Option<i64>,
    /// IP container di `network` yang diminta — `None` kalau container
    /// belum/tidak tersambung ke network itu.
    pub ip_address: Option<String>,
}

pub async fn inspect(
    docker: &Docker,
    container_id: &str,
    network: &str,
) -> Result<ContainerStatus, DockerClientError> {
    let info = call(docker.inspect_container(container_id, None)).await?;

    let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);
    let exit_code = info.state.as_ref().and_then(|s| s.exit_code);
    let ip_address = info
        .network_settings
        .as_ref()
        .and_then(|ns| ns.networks.as_ref())
        .and_then(|nets| nets.get(network))
        .and_then(|ep| ep.ip_address.clone())
        .filter(|ip| !ip.is_empty());

    Ok(ContainerStatus {
        running,
        exit_code,
        ip_address,
    })
}

#[derive(Debug, Clone)]
pub struct ContainerStatsObservation {
    pub cpu_delta: u64,
    pub system_delta: u64,
    pub online_cpus: u32,
    pub memory_usage: u64,
    pub inactive_file: u64,
    pub memory_max: u64,
    pub memory_limit: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub restart_count: i64,
}

/// Ambil satu snapshot stats Docker tanpa menunggu stream berkelanjutan.
/// `stream=false` + `one-shot=true` penting agar satu container tidak
/// menahan worker metrik lebih lama dari timeout tahap ini.
pub async fn stats(
    docker: &Docker,
    container_id: &str,
) -> Result<ContainerStatsObservation, DockerClientError> {
    let options = StatsOptionsBuilder::default()
        .stream(false)
        .one_shot(true)
        .build();
    let mut stream = docker.stats(container_id, Some(options));
    let response = match tokio::time::timeout(CLIENT_CALL_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(response))) => response,
        Ok(Some(Err(err))) => {
            tracing::warn!(error = %err, "gagal membaca stats container Docker");
            return Err(DockerClientError::Unreachable);
        }
        Ok(None) => {
            return Err(DockerClientError::Other(
                "stats container kosong".to_string(),
            ));
        }
        Err(_) => return Err(DockerClientError::Timeout),
    };
    let cpu = response
        .cpu_stats
        .as_ref()
        .ok_or_else(|| DockerClientError::Other("stats CPU container kosong".to_string()))?;
    let usage = cpu
        .cpu_usage
        .as_ref()
        .ok_or_else(|| DockerClientError::Other("penggunaan CPU container kosong".to_string()))?;
    let previous_cpu = response
        .precpu_stats
        .as_ref()
        .ok_or_else(|| DockerClientError::Other("stats CPU sebelumnya kosong".to_string()))?;
    let previous_usage = previous_cpu
        .cpu_usage
        .as_ref()
        .ok_or_else(|| DockerClientError::Other("penggunaan CPU sebelumnya kosong".to_string()))?;
    let current_total = usage
        .total_usage
        .ok_or_else(|| DockerClientError::Other("counter CPU container kosong".to_string()))?;
    let previous_total = previous_usage
        .total_usage
        .ok_or_else(|| DockerClientError::Other("counter CPU sebelumnya kosong".to_string()))?;
    let current_system = cpu.system_cpu_usage.ok_or_else(|| {
        DockerClientError::Other("counter system CPU container kosong".to_string())
    })?;
    let previous_system = previous_cpu.system_cpu_usage.ok_or_else(|| {
        DockerClientError::Other("counter system CPU sebelumnya kosong".to_string())
    })?;
    let cpu_delta = current_total.saturating_sub(previous_total);
    let system_delta = current_system.saturating_sub(previous_system);
    if system_delta == 0 {
        return Err(DockerClientError::Other(
            "delta system CPU container kosong".to_string(),
        ));
    }
    let memory = response
        .memory_stats
        .as_ref()
        .ok_or_else(|| DockerClientError::Other("stats memori container kosong".to_string()))?;
    let interfaces = response
        .networks
        .as_ref()
        .ok_or_else(|| DockerClientError::Other("network stats container kosong".to_string()))?;
    let (net_rx, net_tx) = interfaces
        .values()
        .try_fold((0_u64, 0_u64), |(rx, tx), item| {
            let item_rx = item.rx_bytes.ok_or_else(|| {
                DockerClientError::Other("network rx container kosong".to_string())
            })?;
            let item_tx = item.tx_bytes.ok_or_else(|| {
                DockerClientError::Other("network tx container kosong".to_string())
            })?;
            Ok::<_, DockerClientError>((rx.saturating_add(item_rx), tx.saturating_add(item_tx)))
        })?;
    let inactive_file = memory
        .stats
        .as_ref()
        .and_then(|values| values.get("inactive_file").copied())
        .ok_or_else(|| {
            DockerClientError::Other("inactive_file stats container kosong".to_string())
        })?;
    let inspect = call(docker.inspect_container(container_id, None)).await?;
    let online_cpus = cpu.online_cpus.ok_or_else(|| {
        DockerClientError::Other("jumlah CPU online container kosong".to_string())
    })?;
    let memory_usage = memory.usage.ok_or_else(|| {
        DockerClientError::Other("penggunaan memori container kosong".to_string())
    })?;
    let memory_max = memory
        .max_usage
        .ok_or_else(|| DockerClientError::Other("memori maksimum container kosong".to_string()))?;
    let memory_limit = memory
        .limit
        .ok_or_else(|| DockerClientError::Other("limit memori container kosong".to_string()))?;
    let restart_count = inspect
        .restart_count
        .ok_or_else(|| DockerClientError::Other("counter restart container kosong".to_string()))?;
    Ok(ContainerStatsObservation {
        cpu_delta,
        system_delta,
        online_cpus,
        memory_usage,
        inactive_file,
        memory_max,
        memory_limit,
        net_rx,
        net_tx,
        restart_count,
    })
}

/// Ambil `tail_lines` baris terakhir stdout+stderr — dipakai menangkap log
/// container yang gagal SEBELUM dihapus (invariant §3 no.7). Bukan
/// streaming log runtime (Fase 3) — satu panggilan, batas waktu pendek.
pub async fn container_logs(
    docker: &Docker,
    container_id: &str,
    tail_lines: u32,
) -> Result<String, DockerClientError> {
    let options = LogsOptionsBuilder::default()
        .stdout(true)
        .stderr(true)
        .tail(&tail_lines.to_string())
        .build();
    let mut stream = docker.logs(container_id, Some(options));

    let hasil = tokio::time::timeout(LOGS_TIMEOUT, async {
        let mut keluaran = String::new();
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => keluaran.push_str(&chunk.to_string()),
                Err(err) => {
                    tracing::warn!(error = %err, "gagal membaca sebagian log container");
                    break;
                }
            }
        }
        keluaran
    })
    .await;

    hasil.map_err(|_| DockerClientError::Timeout)
}

/// Kegagalan membuka stream `docker logs --follow`. Dipisah dari
/// [`DockerClientError`] karena kontrak HTTP memetakan dua kasusnya ke status
/// BERBEDA (`docs/api-contract.md` `GET /events/log/runtime/{id}`), dan
/// pemanggil harus bisa membedakannya TANPA mem-parsing string pesan.
#[derive(Debug)]
pub enum LogFollowError {
    /// Docker membalas 404 untuk container itu — sudah dihapus di server
    /// target. Kontrak: 502 + pesan "container sudah tidak ada".
    ContainerHilang,
    /// Chunk pertama tidak datang dalam [`LOGS_FOLLOW_FIRST_CHUNK_TIMEOUT`].
    /// Kontrak: 504. BUKAN dipakai untuk sunyi setelah chunk pertama.
    TimeoutChunkPertama,
    /// Socket tidak terjangkau atau daemon menolak permintaan.
    Unreachable,
}

/// Petakan error `bollard` saat membuka stream log ke kategori. Fungsi murni
/// supaya bisa diuji tanpa Docker.
fn petakan_error_log_follow(err: &bollard::errors::Error) -> LogFollowError {
    match err {
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        } => LogFollowError::ContainerHilang,
        _ => LogFollowError::Unreachable,
    }
}

/// `docker logs --follow --timestamps` untuk viewer log runtime (Fase 3).
///
/// Mengembalikan chunk pertama yang sudah terbaca DITAMBAH sisa stream, bukan
/// hanya stream: batas 15 detik hanya berlaku untuk chunk pertama, jadi chunk
/// itu harus sudah ditarik sebelum fungsi ini kembali. Bentuk `(pertama, sisa)`
/// membuat kesalahan "membungkus seluruh stream dengan timeout" tidak mungkin
/// dilakukan pemanggil secara tidak sengaja — sisa stream tidak membawa batas
/// waktu apa pun.
///
/// Byte log diteruskan APA ADANYA, termasuk escape ANSI: penanggalan ANSI dan
/// escaping HTML sama-sama tugas `src/web/**`
/// (`web::logs::tanggalkan_ansi`). Modul ini tidak menyaring, tidak memotong,
/// tidak menanggalkan apa pun.
///
/// `tail_lines` diteruskan apa adanya — penjepitan ke batas maksimum 2000
/// dilakukan SEKALI di handler (`routes/**`) yang memvalidasi query, bukan di
/// sini, supaya batas kontrak HTTP tidak tersebar di dua tempat.
pub async fn container_logs_follow(
    docker: &Docker,
    container_id: &str,
    tail_lines: u32,
) -> Result<
    (
        String,
        impl tokio_stream::Stream<Item = Result<String, LogFollowError>>,
    ),
    LogFollowError,
> {
    let options = LogsOptionsBuilder::default()
        .follow(true)
        .stdout(true)
        .stderr(true)
        .timestamps(true)
        .tail(&tail_lines.to_string())
        .build();
    let mut stream = docker.logs(container_id, Some(options));

    let pertama = match tokio::time::timeout(LOGS_FOLLOW_FIRST_CHUNK_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(chunk))) => chunk.to_string(),
        Ok(Some(Err(err))) => {
            tracing::warn!(error = %err, "gagal membuka stream log runtime container");
            return Err(petakan_error_log_follow(&err));
        }
        // Stream berakhir tanpa satu chunk pun: container ada tapi belum
        // pernah menulis apa pun. Bukan error — pemanggil merender state
        // "belum ada keluaran".
        Ok(None) => String::new(),
        Err(_) => return Err(LogFollowError::TimeoutChunkPertama),
    };

    let sisa = stream.map(|item| {
        item.map(|chunk| chunk.to_string()).map_err(|err| {
            tracing::warn!(error = %err, "stream log runtime container terputus");
            petakan_error_log_follow(&err)
        })
    });

    Ok((pertama, sisa))
}

/// `docker stop --time={grace_secs}` — flag WAJIB eksplisit (default docker
/// 10 detik bentrok dengan tabel drain `docs/plan.md`).
pub async fn stop_container(
    docker: &Docker,
    container_id: &str,
    grace_secs: i32,
) -> Result<(), DockerClientError> {
    let options = StopContainerOptionsBuilder::default().t(grace_secs).build();
    call_dengan_timeout(
        STOP_CONTAINER_TIMEOUT,
        docker.stop_container(container_id, Some(options)),
    )
    .await
}

pub async fn remove_container(
    docker: &Docker,
    container_id: &str,
) -> Result<(), DockerClientError> {
    let options = RemoveContainerOptionsBuilder::default().force(true).build();
    call(docker.remove_container(container_id, Some(options))).await
}

/// `true` kalau ADA container (jalan atau berhenti — `all(true)`) berlabel
/// `label` (bentuk `"key=value"`). Dipakai bootstrap Traefik lazy
/// (`docs/plan.md` Q2) — cek murah, dijalankan tiap deploy, bukan hanya
/// "deploy pertama": self-healing kalau container Traefik pernah dihapus
/// manual, tanpa perlu state tambahan yang melacak "sudah pernah bootstrap".
///
/// // ponytail: batasnya — HANYA cek keberadaan label, bukan status `running`.
/// Kalau container Traefik pernah dibuat tapi gagal start (kasus langka:
/// deploy pertama mati tepat di antara create dan start), pemeriksaan ini
/// akan salah mengira bootstrap sudah selesai dan deploy app berikutnya
/// diam-diam tidak punya proxy yang jalan. Upgrade: tambahkan cek `running`
/// lewat `inspect` di sini kalau ini pernah kejadian nyata (drift Traefik
/// mati adalah kandidat kuat untuk banner drift Fase 4, bukan auto-heal).
pub async fn container_exists_with_label(
    docker: &Docker,
    label: &str,
) -> Result<bool, DockerClientError> {
    Ok(!list_containers_with_label(docker, label).await?.is_empty())
}

#[derive(Debug, Clone)]
pub struct ContainerObservation {
    pub id: String,
    pub image: Option<String>,
    pub labels: HashMap<String, String>,
    pub running: bool,
    pub status: Option<String>,
}

/// Baca metadata container berlabel tanpa mengubah kondisi server.
/// Hanya field yang diperlukan rekonsiliasi dikembalikan; response bollard
/// mentah tidak boleh menyeberang ke domain atau UI.
pub async fn list_containers_with_label(
    docker: &Docker,
    label: &str,
) -> Result<Vec<ContainerObservation>, DockerClientError> {
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![label.to_string()]);
    let options = ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    let containers = call(docker.list_containers(Some(options))).await?;
    Ok(containers
        .into_iter()
        .filter_map(|container| {
            let id = container.id?;
            let labels = container.labels.unwrap_or_default();
            let running = labels
                .get("platform.deployment")
                .is_some_and(|_| container.state == Some(ContainerSummaryStateEnum::RUNNING));
            Some(ContainerObservation {
                id,
                image: container.image,
                labels,
                running,
                status: container.status,
            })
        })
        .collect())
}

/// Buat network `name` kalau belum ada — 409 (sudah ada) dianggap sukses,
/// bukan error, supaya dipanggil idempoten tiap deploy tanpa cek dulu.
pub async fn ensure_network(docker: &Docker, name: &str) -> Result<(), DockerClientError> {
    let config = NetworkCreateRequest {
        name: name.to_string(),
        ..Default::default()
    };
    match tokio::time::timeout(CLIENT_CALL_TIMEOUT, docker.create_network(config)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 409, ..
        })) => Ok(()),
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "gagal membuat docker network");
            Err(DockerClientError::Unreachable)
        }
        Err(_) => Err(DockerClientError::Timeout),
    }
}

/// Resolusi referensi image (bisa berupa TAG, mis. hasil pull bootstrap
/// Traefik) ke digest lengkap pertama di `RepoDigests` — dipakai supaya
/// container infra tetap dijalankan dengan referensi `@sha256:...`
/// (invariant §5 no.6 berlaku juga untuk image infra, bukan hanya app;
/// tag hanya dipakai sebagai ARGUMEN pull, tidak pernah untuk `create_container`).
#[derive(Debug, Clone)]
pub struct ImageObservation {
    pub id: String,
    pub repo_digests: Vec<String>,
    pub repo_tags: Vec<String>,
    pub containers: i64,
}

pub async fn list_images(docker: &Docker) -> Result<Vec<ImageObservation>, DockerClientError> {
    let options = ListImagesOptionsBuilder::default().all(true).build();
    let images = call(docker.list_images(Some(options))).await?;
    Ok(images
        .into_iter()
        .map(|image| ImageObservation {
            id: image.id,
            repo_digests: image.repo_digests,
            repo_tags: image.repo_tags,
            containers: image.containers,
        })
        .collect())
}

pub async fn remove_image(docker: &Docker, image: &str) -> Result<(), DockerClientError> {
    let options = RemoveImageOptionsBuilder::default().noprune(true).build();
    call_dengan_timeout(
        REMOVE_IMAGE_TIMEOUT,
        docker.remove_image(image, Some(options), None),
    )
    .await
    .map(|_| ())
}

pub async fn resolve_image_digest(
    docker: &Docker,
    image_ref: &str,
) -> Result<String, DockerClientError> {
    let info = call(docker.inspect_image(image_ref)).await?;
    info.repo_digests
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| {
            DockerClientError::Other("image tidak punya RepoDigests setelah pull".to_string())
        })
}

/// Container Traefik tunggal per server — port 80 host-bound (BEDA dari
/// container app yang tidak pernah `-p`, invariant §5 no.5: Traefik ADALAH
/// pintu masuk dari luar, bukan salah satu dari dua container app yang hidup
/// bersamaan). Socket docker di-mount read-only untuk provider docker label
/// discovery (CLAUDE.md §4 "Proxy di target: Traefik, Docker label
/// discovery"). TLS/ACME belum disambung — di luar scope Fase 2
/// (`docs/plan.md` task ini fokus loop deploy, bukan sertifikat).
pub async fn create_traefik_container(
    docker: &Docker,
    image_ref: &str,
    network: &str,
) -> Result<String, DockerClientError> {
    let options = CreateContainerOptionsBuilder::default()
        .name(TRAEFIK_CONTAINER_NAME)
        .build();

    let mut labels = HashMap::new();
    labels.insert("platform.traefik".to_string(), "true".to_string());

    let mut port_bindings = HashMap::new();
    port_bindings.insert(
        "80/tcp".to_string(),
        Some(vec![PortBinding {
            host_ip: None,
            host_port: Some("80".to_string()),
        }]),
    );

    let body = ContainerCreateBody {
        image: Some(image_ref.to_string()),
        labels: Some(labels),
        exposed_ports: Some(vec!["80/tcp".to_string()]),
        cmd: Some(vec![
            "--providers.docker=true".to_string(),
            "--providers.docker.exposedbydefault=false".to_string(),
            format!("--providers.docker.network={network}"),
            "--entrypoints.web.address=:80".to_string(),
        ]),
        host_config: Some(HostConfig {
            network_mode: Some(network.to_string()),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            port_bindings: Some(port_bindings),
            binds: Some(vec![
                "/var/run/docker.sock:/var/run/docker.sock:ro".to_string(),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let response = call(docker.create_container(Some(options), body)).await?;
    Ok(response.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_server(status_code: u16) -> bollard::errors::Error {
        bollard::errors::Error::DockerResponseServerError {
            status_code,
            message: "pesan mentah yang tidak boleh sampai ke klien".to_string(),
        }
    }

    #[test]
    fn docker_404_dipetakan_ke_container_hilang() {
        assert!(matches!(
            petakan_error_log_follow(&error_server(404)),
            LogFollowError::ContainerHilang
        ));
    }

    #[test]
    fn status_server_lain_bukan_container_hilang() {
        // 409/500/503 bukan "container sudah tidak ada" — kontrak HTTP
        // memetakan keduanya ke status berbeda, jadi menyamakannya akan
        // menampilkan pesan perbaikan yang salah ke pengguna.
        for status in [409, 500, 503] {
            assert!(matches!(
                petakan_error_log_follow(&error_server(status)),
                LogFollowError::Unreachable
            ));
        }
    }

    #[test]
    fn timeout_chunk_pertama_tidak_pernah_dihasilkan_pemetaan_error() {
        // TimeoutChunkPertama HANYA boleh lahir dari batas waktu chunk
        // pertama, bukan dari error bollard apa pun — kalau pemetaan ini
        // pernah mengembalikannya, handler membalas 504 untuk container yang
        // sebenarnya sudah hilang (mestinya 502).
        assert!(!matches!(
            petakan_error_log_follow(&error_server(404)),
            LogFollowError::TimeoutChunkPertama
        ));
        assert!(!matches!(
            petakan_error_log_follow(&bollard::errors::Error::APIVersionParseError {}),
            LogFollowError::TimeoutChunkPertama
        ));
    }

    #[test]
    fn batas_chunk_pertama_lima_belas_detik_dan_bukan_logs_timeout() {
        // Angka dikunci tabel "Timeout per tahap" docs/plan.md. Memakai
        // LOGS_TIMEOUT (10 detik) di jalur follow adalah bug yang disebut
        // eksplisit di plan.md.
        assert_eq!(LOGS_FOLLOW_FIRST_CHUNK_TIMEOUT, Duration::from_secs(15));
        assert_ne!(LOGS_FOLLOW_FIRST_CHUNK_TIMEOUT, LOGS_TIMEOUT);
    }
}
