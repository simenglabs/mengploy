//! `GET/POST /apps`, `GET /apps/baru`, `GET /apps/{id}`,
//! `POST /apps/{id}/domain`, `POST /apps/{id}/token` — `docs/plan.md`
//! Fase 2. Sesi cookie + CSRF, BEDA dari `routes::deploy_api` (bearer).

use std::collections::{HashMap, HashSet};

use anyhow::Context as _;
use axum::Form;
use axum::extract::{Extension, Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::apps::NewApp;
use crate::apps::repo as apps_repo;
use crate::auth::deploy_token;
use crate::auth::session::Session;
use crate::deployments::{
    DeployJobPayload, LOCK_TTL_SECS, NewDeployment, repo as deployments_repo,
};
use crate::error::AppError;
use crate::servers::repo as servers_repo;
use crate::state::AppState;
use crate::web;

use super::servers::{fleet_strip, not_found};

/// Batas riwayat deployment yang ditampilkan tab Deployments. Tanpa paging:
/// PRD menyasar 3-8 server dengan sedikit app (`docs/prd.md:12`); kalau angka
/// ini jadi masalah nyata, itu keputusan fase lain (`docs/plan.md`).
const BATAS_RIWAYAT_DEPLOYMENT: usize = 100;

const PESAN_CSRF_TIDAK_VALID: &str =
    "Sesi tidak valid atau kedaluwarsa. Silakan muat ulang halaman dan coba lagi.";

/// `GET /apps` — daftar app.
pub async fn daftar(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Result<Response, AppError> {
    let apps = apps_repo::list_ringkas(&state.db_read).await?;
    let servers = servers_repo::list_ringkas(&state.db_read).await?;
    let strip = fleet_strip(&state).await?;

    Ok(web::render_apps(&apps, &servers, &session.csrf_token, Some(strip)).into_response())
}

/// `GET /apps/baru` — form tambah app.
pub async fn app_baru_form(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Result<Response, AppError> {
    let servers = servers_repo::list_ringkas(&state.db_read).await?;
    let strip = fleet_strip(&state).await?;

    Ok(web::render_app_baru(&servers, &session.csrf_token, None, Some(strip)).into_response())
}

#[derive(Deserialize)]
pub struct AppBaruForm {
    csrf_token: String,
    server_id: String,
    name: String,
    port: i64,
    health_path: String,
    health_grace_secs: i64,
    /// Referensi repo Git (opsional, murni metadata — PRD §1.5: mengploy
    /// tidak pernah build image, CI yang membangun).
    repo_url: Option<String>,
}

/// `POST /apps` — validasi, simpan, redirect ke detail.
pub async fn app_baru_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Form(form): Form<AppBaruForm>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }
    if let Err(pesan) = validasi_app_baru(&form) {
        let servers = servers_repo::list_ringkas(&state.db_read).await?;
        let strip = fleet_strip(&state).await?;
        let body = web::render_app_baru(&servers, &session.csrf_token, Some(pesan), Some(strip));
        return Ok((axum::http::StatusCode::BAD_REQUEST, body).into_response());
    }

    let health_path = if form.health_path.trim().is_empty() {
        "/health".to_string()
    } else {
        form.health_path.trim().to_string()
    };

    // Referensi repo opsional — dipakai untuk tautan dan generator workflow
    // CI. TIDAK pernah dipakai untuk clone/build (PRD §1.5).
    let repo_url = form
        .repo_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty());

    let id = match apps_repo::insert(
        &state.db_write,
        NewApp {
            server_id: &form.server_id,
            name: form.name.trim(),
            health_path: &health_path,
            health_grace_secs: form.health_grace_secs,
            port: form.port,
            // Invariant §5 no.5 — SEMUA container `unless-stopped`, tidak
            // ada opsi lain, jadi tidak ada input form untuk ini.
            restart_policy: "unless-stopped",
            repo_url,
        },
    )
    .await
    {
        Ok(id) => id,
        Err(err) if err.downcast_ref::<apps_repo::ServerLocked>().is_some() => {
            return Err(AppError::Conflict(
                "Server sedang menjalankan operasi armada; pembuatan app dicoba lagi setelah operasi selesai.".to_string(),
            ));
        }
        Err(err) => return Err(err.into()),
    };

    Ok(Redirect::to(&format!("/apps/{id}")).into_response())
}
fn validasi_app_baru(form: &AppBaruForm) -> Result<(), &'static str> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("Nama app wajib diisi.");
    }
    if !nama_app_valid(name) {
        return Err(
            "Nama app hanya boleh huruf, angka, titik, garis bawah, dan tanda hubung \
             (a-z, 0-9, ., _, -). Langkah perbaikan: Hindari spasi dan karakter khusus \
             karena nama dipakai sebagai nama container Docker.",
        );
    }
    if form.server_id.trim().is_empty() {
        return Err("Server wajib dipilih.");
    }
    if !(1..=65535).contains(&form.port) {
        return Err("Port harus berupa angka bulat dalam rentang 1 - 65535.");
    }
    if form.health_grace_secs < 0 {
        return Err("Grace period tidak boleh negatif.");
    }
    if let Some(url) = form
        .repo_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        && !repo_url_valid(url)
    {
        return Err(
            "URL repo tidak valid. Langkah perbaikan: Gunakan URL lengkap \
             https://github.com/org/repo atau https://gitlab.com/org/repo.",
        );
    }
    Ok(())
}

/// Nama app dibatasi ke karakter yang sah untuk nama container Docker
/// (`[a-zA-Z0-9][a-zA-Z0-9_.-]*`) — sekaligus menutup injeksi shell/YAML
/// lewat generator workflow (kutip tunggal, kutip ganda, backslash, newline
/// tidak pernah bisa masuk).
fn nama_app_valid(nama: &str) -> bool {
    !nama.is_empty()
        && nama
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// URL repo Git: harus HTTPS dengan host non-kosong dan ada path repo —
/// murni metadata referensi (tidak pernah di-fetch mengploy, PRD §1.5),
/// jadi cukup memastikan bentuk URL yang masuk akal, bukan daftar host
/// tertutup yang bisa menolak GitLab self-hosted.
fn repo_url_valid(url: &str) -> bool {
    if url.contains(char::is_whitespace) {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    !host.is_empty() && !path.is_empty()
}

/// `GET /apps/{id}/workflow/{jenis}` — unduh workflow CI contoh (isinya
/// sudah terisi nama app). `jenis` = `github` atau `gitlab`. Mengembalikan
/// file teks YAML dengan `Content-Disposition: attachment` supaya browser
/// mengunduhnya (bukan merender), sesuai pola unduh log.
pub async fn workflow_unduh(
    State(state): State<AppState>,
    Path((id, jenis)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let app = apps_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or_else(not_found)?;

    let (nama_file, isi) = match jenis.as_str() {
        "github" => (
            crate::workflows::GITHUB_ACTIONS_PATH,
            crate::workflows::github_actions_workflow(&app.name),
        ),
        "gitlab" => (
            crate::workflows::GITLAB_CI_PATH,
            crate::workflows::gitlab_ci_workflow(&app.name),
        ),
        _ => return Err(not_found()),
    };

    let nama_unduh = nama_file.rsplit('/').next().unwrap_or(nama_file);
    Ok((
        [(header::CONTENT_TYPE, "text/yaml")],
        [(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{nama_unduh}\""),
        )],
        isi,
    )
        .into_response())
}

/// `GET /apps/{id}` — overview: konfigurasi, domain, token, riwayat.
pub async fn detail(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let body = render_detail(&state, &session, &id, None).await?;
    Ok(body.into_response())
}

async fn render_detail(
    state: &AppState,
    session: &Session,
    id: &str,
    token_baru: Option<&str>,
) -> Result<maud::Markup, AppError> {
    let app = apps_repo::find_by_id(&state.db_read, id)
        .await?
        .ok_or_else(not_found)?;
    let server = servers_repo::find_ringkas_by_id(&state.db_read, &app.server_id).await?;
    let server_name = server.map(|s| s.name).unwrap_or_else(|| "-".to_string());
    let domains = apps_repo::list_domains(&state.db_read, id).await?;
    let tokens = apps_repo::list_deploy_tokens_ringkas(&state.db_read, id).await?;
    let deploys = deployments_repo::list_by_app(&state.db_read, id).await?;
    let strip = fleet_strip(state).await?;

    Ok(web::render_app_detail(
        &app,
        &server_name,
        &domains,
        &tokens,
        &deploys,
        token_baru,
        &session.csrf_token,
        Some(strip),
    ))
}

/// `GET /apps/{id}/deployments` — tab Deployments (riwayat).
///
/// Hanya MEMBACA. Tidak ada tombol rollback di sini — itu Fase 5
/// (`docs/prd.md:326`). Batas 100 deployment terbaru tanpa paging; frontend
/// merender penanda "menampilkan 100 terbaru" berdasar flag `dipotong`.
pub async fn tab_deployments(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let app = apps_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or_else(not_found)?;
    let mut deploys = deployments_repo::list_by_app(&state.db_read, &id).await?;
    let dipotong = deploys.len() > BATAS_RIWAYAT_DEPLOYMENT;
    deploys.truncate(BATAS_RIWAYAT_DEPLOYMENT);
    let strip = fleet_strip(&state).await?;

    Ok(crate::web::render_app_tab_deployments(
        &app,
        &deploys,
        dipotong,
        &session.csrf_token,
        Some(strip),
    )
    .into_response())
}

/// `GET /apps/{id}/logs` — tab Logs (halaman saja).
///
/// Handler ini TIDAK membuka SSH dan TIDAK membuka socket forward — itu
/// terjadi di endpoint SSE (`routes::events`) dan di `/apps/{id}/logs/isi`.
/// Tidak ada deployment live / `container_id` NULL → state "belum ada
/// container yang berjalan", SSE tidak dipasang, tetap 200 (bukan 404,
/// bukan 500).
pub async fn tab_logs(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let app = apps_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or_else(not_found)?;
    let live = deployments_repo::find_current_live(&state.db_read, &id, "").await?;
    let ada_container = live.as_ref().is_some_and(|d| d.container_id.is_some());
    let strip = fleet_strip(&state).await?;

    Ok(
        crate::web::render_app_tab_logs(&app, ada_container, &session.csrf_token, Some(strip))
            .into_response(),
    )
}

#[derive(Deserialize)]
pub struct DomainBaruForm {
    csrf_token: String,
    host: String,
}

/// `POST /apps/{id}/domain`.
pub async fn domain_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Form(form): Form<DomainBaruForm>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }
    apps_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or_else(not_found)?;

    let host = form.host.trim();
    if host.is_empty() {
        return Err(AppError::BadRequest("Domain wajib diisi.".to_string()));
    }

    apps_repo::add_domain(&state.db_write, &id, host).await?;

    Ok(Redirect::to(&format!("/apps/{id}")).into_response())
}

#[derive(Deserialize)]
pub struct TokenBaruForm {
    csrf_token: String,
    name: String,
}

/// `POST /apps/{id}/token` — plaintext token HANYA muncul di response INI,
/// tidak pernah lagi (invariant §5 no.11). Render detail langsung (bukan
/// redirect) supaya plaintext tidak pernah singgah di query string/riwayat
/// browser.
pub async fn token_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Form(form): Form<TokenBaruForm>,
) -> Result<Response, AppError> {
    if form.csrf_token != session.csrf_token {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }
    apps_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or_else(not_found)?;

    let name = form.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("Nama token wajib diisi.".to_string()));
    }

    let plaintext = deploy_token::generate();
    let hash = deploy_token::hash(&plaintext).map_err(AppError::from)?;
    apps_repo::insert_deploy_token(&state.db_write, &id, name, &hash).await?;

    let body = render_detail(&state, &session, &id, Some(&plaintext)).await?;
    Ok(body.into_response())
}

/// Jumlah slot baris tambah inline yang SELALU dirender kosong di tab
/// Environment — `docs/plan.md` Fase 4: batasan disengaja supaya form bisa
/// dibaca lewat nama field TETAP (`new_key_0..N`), tanpa JS dinamis
/// (PRD non-goal: JS di luar HTMX/xterm). Kalau ini kurang di pemakaian
/// nyata, submit ulang lagi setelah baris tersimpan cukup — bukan batas
/// keras pada jumlah env var per app.
pub const ENV_NEW_ROW_SLOTS: usize = 5;

/// `GET /apps/{id}/env` — tab Environment.
pub async fn tab_environment(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let body = render_environment(&state, &session, &id, None, &[]).await?;
    Ok(body.into_response())
}

async fn render_environment(
    state: &AppState,
    session: &Session,
    id: &str,
    pesan: Option<&str>,
    diffs: &[web::EnvDiff],
) -> Result<maud::Markup, AppError> {
    let app = apps_repo::find_by_id(&state.db_read, id)
        .await?
        .ok_or_else(not_found)?;
    let env_raw = apps_repo::list_env_vars_encrypted(&state.db_read, id).await?;
    // Dekripsi HANYA baris non-secret — keputusan "tampilkan atau topengi"
    // (PRD Fase 4 baris Frontend: "field secret bertopeng dengan tombol
    // Replace") diambil DI SINI, satu-satunya tempat `apps::repo` dan
    // `state.crypto` sama-sama terlihat untuk fitur ini.
    let mut env_vars = Vec::with_capacity(env_raw.len());
    for (key, value_encrypted, is_secret) in env_raw {
        let value_plaintext = if is_secret {
            None
        } else {
            Some(state.crypto.decrypt(&value_encrypted)?)
        };
        env_vars.push(web::EnvVarTampil {
            key,
            value_plaintext,
            is_secret,
        });
    }
    let strip = fleet_strip(state).await?;

    Ok(crate::web::render_app_tab_environment(
        &app,
        &env_vars,
        pesan,
        diffs,
        ENV_NEW_ROW_SLOTS,
        &session.csrf_token,
        Some(strip),
    ))
}

/// `POST /apps/{id}/env` — validasi, enkripsi, snapshot baru, redeploy
/// dengan digest yang sama (`docs/plan.md` Fase 4 "Desain teknis").
///
/// Form diterima sebagai `HashMap<String, String>` (bukan struct tetap):
/// baris env var yang sudah ada dan baris baru sama-sama punya nama field
/// yang ditentukan oleh KEY-nya sendiri (`value__{key}`,
/// `delete__{key}`) atau indeks slot tetap (`new_key_{i}`, dst) — jumlah
/// kolom tidak diketahui saat compile, HashMap adalah representasi paling
/// jujur untuk itu (bukan kemalasan menghindari struct, `Form<HashMap<..>>`
/// memang cara `serde_urlencoded` menangani field dinamis).
pub async fn env_submit(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    if form.get("csrf_token").map(String::as_str) != Some(session.csrf_token.as_str()) {
        return Err(AppError::BadRequest(PESAN_CSRF_TIDAK_VALID.to_string()));
    }
    apps_repo::find_by_id(&state.db_read, &id)
        .await?
        .ok_or_else(not_found)?;

    let existing_raw = apps_repo::list_env_vars_encrypted(&state.db_read, &id).await?;
    let existing: Vec<(String, bool)> = existing_raw
        .iter()
        .map(|(key, _value_encrypted, is_secret)| (key.clone(), *is_secret))
        .collect();
    let existing_keys: HashSet<&str> = existing.iter().map(|(key, _)| key.as_str()).collect();

    enum Perubahan {
        Hapus(String),
        Kosongkan {
            key: String,
            is_secret: bool,
        },
        Set {
            key: String,
            value: String,
            is_secret: bool,
        },
    }
    let mut perubahan = Vec::new();

    for (key, is_secret) in &existing {
        let hapus = form.contains_key(&format!("delete__{key}"));
        let kosongkan = form.contains_key(&format!("empty__{key}"));
        if hapus && kosongkan {
            return Err(AppError::BadRequest(format!(
                "Pilih hanya hapus atau set value menjadi kosong untuk {key}."
            )));
        }
        if hapus {
            perubahan.push(Perubahan::Hapus(key.clone()));
            continue;
        }
        if kosongkan {
            perubahan.push(Perubahan::Kosongkan {
                key: key.clone(),
                is_secret: *is_secret,
            });
            continue;
        }
        let raw = form
            .get(&format!("value__{key}"))
            .map(String::as_str)
            .unwrap_or_default();
        // Field kosong = "tidak diubah" (pola sama tombol Replace untuk
        // secret: kosong berarti pertahankan value lama). Ini simplifikasi
        // sadar — env value yang MEMANG harus kosong tidak bisa diset lewat
        // form ini, cuma lewat hapus lalu (kalau perlu) dibuat lagi.
        if raw.is_empty() {
            continue;
        }
        if raw.contains('\n') || raw.contains('\r') {
            return Err(AppError::BadRequest(format!(
                "Nilai untuk {key} tidak boleh mengandung baris baru."
            )));
        }
        perubahan.push(Perubahan::Set {
            key: key.clone(),
            value: raw.to_string(),
            is_secret: *is_secret,
        });
    }

    let mut key_baru_terlihat: HashSet<String> = HashSet::new();
    for i in 0..ENV_NEW_ROW_SLOTS {
        let key = form
            .get(&format!("new_key_{i}"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if key.is_empty() {
            continue;
        }
        if existing_keys.contains(key.as_str()) {
            return Err(AppError::BadRequest(format!(
                "Key {key} sudah ada — edit baris yang sudah ada, bukan menambah baris baru."
            )));
        }
        if !key_baru_terlihat.insert(key.clone()) {
            return Err(AppError::BadRequest(format!(
                "Key {key} muncul di lebih dari satu baris baru."
            )));
        }
        let value = form
            .get(&format!("new_value_{i}"))
            .map(String::as_str)
            .unwrap_or_default();
        if value.contains('\n') || value.contains('\r') {
            return Err(AppError::BadRequest(format!(
                "Nilai untuk {key} tidak boleh mengandung baris baru."
            )));
        }
        let is_secret = form.contains_key(&format!("new_secret_{i}"));
        perubahan.push(Perubahan::Set {
            key,
            value: value.to_string(),
            is_secret,
        });
    }

    if perubahan.is_empty() {
        let body = render_environment(&state, &session, &id, None, &[]).await?;
        return Ok(body.into_response());
    }

    let diffs: Vec<web::EnvDiff> = perubahan
        .iter()
        .map(|perubahan| match perubahan {
            Perubahan::Hapus(key) => web::EnvDiff {
                key: key.clone(),
                kind: web::EnvDiffKind::Deleted,
                is_secret: existing
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, s)| *s)
                    .unwrap_or(false),
            },
            Perubahan::Kosongkan { key, is_secret } => web::EnvDiff {
                key: key.clone(),
                kind: web::EnvDiffKind::Emptied,
                is_secret: *is_secret,
            },
            Perubahan::Set { key, is_secret, .. } => web::EnvDiff {
                key: key.clone(),
                kind: if existing_keys.contains(key.as_str()) {
                    web::EnvDiffKind::Changed
                } else {
                    web::EnvDiffKind::Added
                },
                is_secret: *is_secret,
            },
        })
        .collect();

    let versi_sebelumnya = apps_repo::find_latest_env_version(&state.db_read, &id).await?;
    let versi_baru = versi_sebelumnya.map(|v| v.version + 1).unwrap_or(1);
    let env_version_id = apps_repo::generate_id();

    // Redeploy dengan digest yang SEDANG live — app yang belum pernah
    // dideploy tidak punya digest untuk dipakai ulang, env tetap tersimpan
    // (di atas), redeploy dilewati dengan pesan jujur.
    let live = deployments_repo::find_current_live(&state.db_read, &id, "").await?;

    // Lock diambil (kalau relevan) SEBELUM membuka transaksi — `db_write`
    // punya `max_connections(1)` (CLAUDE.md §7), jadi transaksi terbuka
    // yang MENAHAN satu-satunya koneksi lalu memanggil `acquire_lock` (yang
    // minta koneksi LAIN dari pool yang sama) akan macet menunggu koneksi
    // yang tidak pernah bebas. Pola sama `routes/deploy_api.rs`: lock dulu
    // via pool, transaksi belakangan.
    let deployment_id_terkunci: Option<String> = match &live {
        None => None,
        Some(_) => {
            let deployment_id = deployments_repo::generate_id();
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            let terkunci =
                apps_repo::acquire_lock(&state.db_write, &id, &deployment_id, now + LOCK_TTL_SECS)
                    .await?;
            terkunci.then_some(deployment_id)
        }
    };

    let mut tx = state
        .db_write
        .begin()
        .await
        .context("mulai transaksi simpan env")?;

    for p in &perubahan {
        match p {
            Perubahan::Hapus(key) => {
                apps_repo::delete_env_var_tx(&mut tx, &id, key).await?;
            }
            Perubahan::Kosongkan { key, is_secret } => {
                let encrypted = state.crypto.encrypt("")?;
                apps_repo::upsert_env_var_tx(&mut tx, &id, key, &encrypted, *is_secret).await?;
            }
            Perubahan::Set {
                key,
                value,
                is_secret,
            } => {
                let encrypted = state.crypto.encrypt(value)?;
                apps_repo::upsert_env_var_tx(&mut tx, &id, key, &encrypted, *is_secret).await?;
            }
        }
    }

    let semua = sqlx::query!(
        r#"SELECT key, value_encrypted, is_secret as "is_secret: bool"
           FROM env_vars WHERE app_id = ? ORDER BY key ASC"#,
        id
    )
    .fetch_all(&mut *tx)
    .await
    .context("baca env var dalam transaksi")?;
    let mut snapshot_map = std::collections::BTreeMap::new();
    for row in semua {
        let plaintext = state.crypto.decrypt(&row.value_encrypted)?;
        snapshot_map.insert(row.key, plaintext);
    }
    let snapshot_json = serde_json::to_string(&snapshot_map).context("serialisasi snapshot env")?;
    let snapshot_encrypted = state.crypto.encrypt(&snapshot_json)?;

    apps_repo::insert_env_version_tx(
        &mut tx,
        &env_version_id,
        &id,
        versi_baru,
        &snapshot_encrypted,
        None,
    )
    .await?;

    let pesan = match (live, deployment_id_terkunci) {
        (None, _) => {
            tx.commit().await.context("commit transaksi simpan env")?;
            "Environment disimpan. App ini belum pernah dideploy, jadi belum ada deployment untuk diterapkan.".to_string()
        }
        (Some(_), None) => {
            tx.commit().await.context("commit transaksi simpan env")?;
            let body = render_environment(
                &state,
                &session,
                &id,
                Some("Environment disimpan, tapi app sedang dalam proses deploy lain — coba simpan lagi sesaat lagi untuk menerapkannya."),
                &[],
            )
            .await?;
            return Ok((axum::http::StatusCode::CONFLICT, body).into_response());
        }
        (Some(dep_live), Some(deployment_id)) => {
            let job_id = deployments_repo::generate_id();
            let job_payload_json = serde_json::to_string(&DeployJobPayload {
                deployment_id: deployment_id.clone(),
            })
            .context("serialisasi payload job deploy")?;

            if let Err(err) = deployments_repo::insert_queued_dengan_job_tx(
                &mut tx,
                &deployment_id,
                NewDeployment {
                    app_id: &id,
                    commit_sha: &dep_live.commit_sha,
                    git_ref: dep_live.git_ref.as_deref(),
                    image_digest: &dep_live.image_digest,
                    trigger_source: "env",
                    env_version_id: Some(&env_version_id),
                },
                &job_id,
                &job_payload_json,
            )
            .await
            {
                let _ = apps_repo::release_lock(&state.db_write, &id, &deployment_id).await;
                return Err(AppError::from(err));
            }
            tx.commit().await.context("commit transaksi simpan env")?;
            "Environment disimpan — deploy baru dengan image yang sama sedang berjalan.".to_string()
        }
    };

    let body = render_environment(&state, &session, &id, Some(&pesan), &diffs).await?;
    Ok(body.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nama_app_valid_menerima_huruf_angka_titik_garis_bawah_dan_hubung() {
        assert!(nama_app_valid("api"));
        assert!(nama_app_valid("api-gateway"));
        assert!(nama_app_valid("api.gateway"));
        assert!(nama_app_valid("api_gateway"));
        assert!(nama_app_valid("App123"));
    }

    #[test]
    fn nama_app_valid_menolak_karakter_yang_membahayakan_workflow() {
        // Karakter ini bisa merusak payload JSON di generator workflow CI
        // (shell single-quote, kutip ganda, backslash, newline).
        assert!(!nama_app_valid("it's"));
        assert!(!nama_app_valid("na\"ma"));
        assert!(!nama_app_valid("na\\ma"));
        assert!(!nama_app_valid("na\nma"));
        assert!(!nama_app_valid("na ma"));
    }

    #[test]
    fn nama_app_valid_menolak_kosong() {
        assert!(!nama_app_valid(""));
    }

    #[test]
    fn repo_url_valid_menerima_github_gitlab_dan_self_hosted() {
        assert!(repo_url_valid("https://github.com/org/repo"));
        assert!(repo_url_valid("https://gitlab.com/org/repo"));
        assert!(repo_url_valid("https://gitlab.perusahaan.id/org/repo"));
        assert!(repo_url_valid("https://git.example.com/org/repo"));
    }

    #[test]
    fn repo_url_valid_menolak_bentuk_url_yang_tidak_layak() {
        assert!(!repo_url_valid("ftp://github.com/org/repo"));
        assert!(!repo_url_valid("http://github.com/org/repo"));
        assert!(!repo_url_valid("https://github.com"));
        assert!(!repo_url_valid("https://github.com/"));
        assert!(!repo_url_valid("https://github.com/org/repo dengan spasi"));
        assert!(!repo_url_valid("bukan-url"));
    }
}
