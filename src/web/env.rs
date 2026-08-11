//! Tab Environment (`GET/POST /apps/{id}/env`) — `docs/plan.md` Fase 4.

use maud::{Markup, html};

use crate::apps::model::AppRingkas;
use crate::web::layout::{app_shell, base_page};
use crate::web::logs::tab_nav;

/// View-model SATU baris env — **TIDAK PERNAH** dibangun dari value
/// terenkripsi mentah (invariant §3 no.7). `value_plaintext` sengaja
/// `Option`: `None` untuk baris `is_secret=true` (topeng ditampilkan),
/// `Some` untuk baris non-secret (boleh diedit langsung di tempat,
/// `docs/plan.md` Fase 4 "Kriteria selesai"). Keputusan dekripsi ada di
/// `routes/apps.rs` — modul ini murni render dari apa yang sudah diberikan.
pub struct EnvVarTampil {
    pub key: String,
    pub value_plaintext: Option<String>,
    pub is_secret: bool,
}

pub enum EnvDiffKind {
    Added,
    Changed,
    Emptied,
    Deleted,
}

pub struct EnvDiff {
    pub key: String,
    pub kind: EnvDiffKind,
    pub is_secret: bool,
}

/// Jumlah baris tambah inline yang ditampilkan — nama field
/// `new_key_{i}`/`new_value_{i}`/`new_secret_{i}` harus PERSIS cocok
/// dengan yang dibaca `routes::apps::env_submit`.
pub fn render_app_tab_environment(
    app: &AppRingkas,
    env_vars: &[EnvVarTampil],
    pesan: Option<&str>,
    diffs: &[EnvDiff],
    new_row_slots: usize,
    csrf_token: &str,
    strip: Option<Markup>,
) -> Markup {
    let content = html! {
        div.detail-title-row { h1 { "App: " (app.name) } }
        (tab_nav(&app.id, "environment"))

        @if let Some(p) = pesan {
            div.alert.alert-warning { (p) }
        }

        div.alert.alert-warning {
            "Menyimpan mengubah environment dan akan memicu deploy baru ke image yang sama sedang berjalan."
        }

        @if !diffs.is_empty() {
            section aria-labelledby="environment-diff-title" {
                h2 id="environment-diff-title" { "Perubahan environment yang diterapkan" }
                ul {
                    @for diff in diffs {
                        li {
                            @match diff.kind {
                                EnvDiffKind::Added => { "+ " }
                                EnvDiffKind::Changed => { "~ " }
                                EnvDiffKind::Emptied => { "~ " }
                                EnvDiffKind::Deleted => { "− " }
                            }
                            code { (diff.key) }
                            " — "
                            @if diff.is_secret {
                                @match diff.kind {
                                    EnvDiffKind::Added => { "(secret diisi)" }
                                    EnvDiffKind::Changed => { "(secret diubah)" }
                                    EnvDiffKind::Emptied => { "(secret menjadi kosong)" }
                                    EnvDiffKind::Deleted => { "dihapus" }
                                }
                            } @else {
                                @match diff.kind {
                                    EnvDiffKind::Added => { "nilai baru ditambahkan" }
                                    EnvDiffKind::Changed => { "nilai diubah" }
                                    EnvDiffKind::Emptied => { "(kosong)" }
                                    EnvDiffKind::Deleted => { "dihapus" }
                                }
                            }
                        }
                    }
                }
            }
        }

        form method="post" action=(format!("/apps/{}/env", app.id)) {
            input type="hidden" name="csrf_token" value=(csrf_token);

            @if env_vars.is_empty() {
                p { "Belum ada environment variable." }
            } @else {
                table.fleet-table {
                    thead {
                        tr {
                            th scope="col" { "Key" }
                            th scope="col" { "Value" }
                            th scope="col" { "Hapus" }
                        }
                    }
                    tbody {
                        @for v in env_vars {
                            tr {
                                td { code { (v.key) } @if v.is_secret { " " span.badge { "secret" } } }
                                td {
                                    @if v.is_secret {
                                        input
                                            type="text"
                                            name=(format!("value__{}", v.key))
                                            placeholder="•••••••• (kosongkan untuk tidak mengganti)";
                                    } @else {
                                        input
                                            type="text"
                                            name=(format!("value__{}", v.key))
                                            value=(v.value_plaintext.as_deref().unwrap_or_default())
                                            placeholder="(kosongkan untuk tidak mengganti)";
                                    }
                                    label {
                                        input type="checkbox" name=(format!("empty__{}", v.key)) value="1";
                                        " set value menjadi kosong"
                                    }
                                }
                                td {
                                    label {
                                        input type="checkbox" name=(format!("delete__{}", v.key)) value="1";
                                        " hapus"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            h2 { "Tambah Variabel" }
            table.fleet-table {
                thead {
                    tr {
                        th scope="col" { "Key" }
                        th scope="col" { "Value" }
                        th scope="col" { "Secret" }
                    }
                }
                tbody {
                    @for i in 0..new_row_slots {
                        tr {
                            td { input type="text" name=(format!("new_key_{i}")) placeholder="NAMA_VARIABEL"; }
                            td { input type="text" name=(format!("new_value_{i}")); }
                            td {
                                label {
                                    input type="checkbox" name=(format!("new_secret_{i}")) value="1";
                                    " secret"
                                }
                            }
                        }
                    }
                }
            }

            div.field-actions {
                button.btn type="submit" { "Simpan & Deploy" }
            }
        }
    };

    base_page(
        &format!("Environment {} - Mengploy", app.name),
        app_shell(Some(csrf_token), strip, content),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> AppRingkas {
        AppRingkas {
            id: "app-1".to_string(),
            server_id: "srv-1".to_string(),
            name: "api".to_string(),
            health_path: "/health".to_string(),
            health_grace_secs: 30,
            port: 8080,
            restart_policy: "unless-stopped".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn kosong_menampilkan_pesan_belum_ada() {
        let markup =
            render_app_tab_environment(&app(), &[], None, &[], 5, "tok", None).into_string();
        assert!(markup.contains("Belum ada environment variable"));
    }

    #[test]
    fn secret_ditopengi_tanpa_plaintext_di_markup() {
        let env_vars = vec![EnvVarTampil {
            key: "DB_PASSWORD".to_string(),
            value_plaintext: None,
            is_secret: true,
        }];
        let markup =
            render_app_tab_environment(&app(), &env_vars, None, &[], 5, "tok", None).into_string();
        assert!(markup.contains("DB_PASSWORD"));
        assert!(markup.contains("kosongkan untuk tidak mengganti"));
        assert!(!markup.contains("value=\"rahasia"));
    }

    #[test]
    fn non_secret_menampilkan_plaintext_di_input() {
        let env_vars = vec![EnvVarTampil {
            key: "NODE_ENV".to_string(),
            value_plaintext: Some("production".to_string()),
            is_secret: false,
        }];
        let markup =
            render_app_tab_environment(&app(), &env_vars, None, &[], 5, "tok", None).into_string();
        assert!(markup.contains(r#"value="production""#));
    }

    #[test]
    fn baris_tambah_inline_sejumlah_slot_diminta() {
        let markup =
            render_app_tab_environment(&app(), &[], None, &[], 3, "tok", None).into_string();
        assert!(markup.contains("new_key_0"));
        assert!(markup.contains("new_key_2"));
        assert!(!markup.contains("new_key_3"));
    }

    #[test]
    fn sentinel_kosong_dan_diff_secret_tidak_membocorkan_nilai() {
        let env_vars = vec![EnvVarTampil {
            key: "TOKEN".to_string(),
            value_plaintext: None,
            is_secret: true,
        }];
        let diffs = vec![EnvDiff {
            key: "TOKEN".to_string(),
            kind: EnvDiffKind::Changed,
            is_secret: true,
        }];
        let markup = render_app_tab_environment(&app(), &env_vars, None, &diffs, 0, "tok", None)
            .into_string();
        assert!(markup.contains("empty__TOKEN"));
        assert!(markup.contains("(secret diubah)"));
        assert!(!markup.contains("rahasia"));
    }
}
