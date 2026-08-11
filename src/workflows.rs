//! Generator workflow CI contoh (GitHub Actions & GitLab CI) untuk satu app.
//!
//! PRD §1.5 non-goal: "Membangun image sendiri — CI yang membangun." Mengploy
//! TIDAK PERNAH build image — file yang dihasilkan di sini adalah SKELETON
//! yang diunduh pengguna dan di-commit ke repo mereka; isinya sudah terisi
//! nama app (`POST /api/v1/deploy` butuh `app` yang cocok dengan
//! `apps.name`) plus placeholder registry yang harus diganti pengguna.
//!
//! Modul murni tanpa I/O — dipakai `routes::apps` merespon unduhan.
//! Nama app di-escape terhadap `"` dan `\` supaya aman disisipkan ke YAML
//! (nilai bisa dari input pengguna).

/// Karakter `"` dan `\` di-escape untuk nilai YAML bertanda kutip ganda.
fn yaml_escape(app_name: &str) -> String {
    app_name.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Nama file workflow GitHub Actions (relative ke root repo).
pub const GITHUB_ACTIONS_PATH: &str = ".github/workflows/deploy-mengploy.yml";

/// Nama file pipeline GitLab CI (relative ke root repo).
pub const GITLAB_CI_PATH: &str = ".gitlab-ci.yml";

/// Isi `.github/workflows/deploy-mengploy.yml` dengan `app_name` terisi.
pub fn github_actions_workflow(app_name: &str) -> String {
    let app = yaml_escape(app_name);
    format!(
        r#"# Workflow contoh: build image lalu deploy ke Mengploy.
#
# Persiapan sekali:
#   1. Buat token deploy di halaman app Mengploy (tab Overview -> Token Deploy).
#   2. Tambah secret repo GitHub:
#        - MENGPLOY_URL  : URL instance mengploy (mis. https://mengploy.contoh.com)
#        - DEPLOY_TOKEN  : token dari langkah 1
#   3. Ganti GHCR_IMAGE di bawah dengan nama image Anda, mis.
#      ghcr.io/<nama-anda>/<nama-repo> (bisa juga Docker Hub, dst.).
#      Kalau registry privat, tambah secret REGISTRY_USERNAME dan
#      REGISTRY_PASSWORD (langkah login di bawah otomatis memakainya).
#
# Mengploy tidak pernah build image — workflow inilah yang membangun dan
# mengirim digest, sesuai arsitektur PRD.
name: Deploy ke Mengploy

on:
  push:
    branches: [main]
  workflow_dispatch:

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Login registry (opsional, kalau privat)
        run: |
          if [ -n "${{{{ secrets.REGISTRY_USERNAME }}}}" ]; then
            echo "${{{{ secrets.REGISTRY_PASSWORD }}}}" | docker login ghcr.io -u "${{{{ secrets.REGISTRY_USERNAME }}}}" --password-stdin
          fi

      - name: Build image
        run: docker build -t $GHCR_IMAGE:$GITHUB_SHA .
        env:
          GHCR_IMAGE: ghcr.io/<nama-anda>/<nama-repo>

      - name: Push image
        run: docker push $GHCR_IMAGE:$GITHUB_SHA
        env:
          GHCR_IMAGE: ghcr.io/<nama-anda>/<nama-repo>

      - name: Deploy ke Mengploy
        run: |
          DIGEST=$(docker inspect --format='{{{{index .RepoDigests 0}}}}' $GHCR_IMAGE:$GITHUB_SHA)
          curl -fsS -X POST "$MENGPLOY_URL/api/v1/deploy" \
            -H "Authorization: Bearer $DEPLOY_TOKEN" \
            -H "Content-Type: application/json" \
            -d "{{\"app\":\"$APP_NAME\",\"image\":\"$DIGEST\",\"commit\":\"$GITHUB_SHA\",\"ref\":\"$GITHUB_REF_NAME\"}}"
        env:
          GHCR_IMAGE: ghcr.io/<nama-anda>/<nama-repo>
          APP_NAME: "{app}"
          MENGPLOY_URL: ${{{{ secrets.MENGPLOY_URL }}}}
          DEPLOY_TOKEN: ${{{{ secrets.DEPLOY_TOKEN }}}}
"#
    )
}

/// Isi `.gitlab-ci.yml` dengan `app_name` terisi.
pub fn gitlab_ci_workflow(app_name: &str) -> String {
    let app = yaml_escape(app_name);
    format!(
        r#"# Workflow contoh: build image lalu deploy ke Mengploy.
#
# Persiapan sekali:
#   1. Buat token deploy di halaman app Mengploy (tab Overview -> Token Deploy).
#   2. Tambah variable CI/CD di project GitLab:
#        - MENGPLOY_URL : URL instance mengploy (mis. https://mengploy.contoh.com)
#        - DEPLOY_TOKEN : token dari langkah 1
#   3. Ganti REGISTRY_IMAGE di bawah dengan nama image Anda, mis.
#      registry.gitlab.com/<nama-anda>/<nama-repo>.
#
# Mengploy tidak pernah build image — pipeline inilah yang membangun dan
# mengirim digest, sesuai arsitektur PRD.
stages:
  - deploy

deploy:
  stage: deploy
  image: docker:27
  services:
    - docker:27-dind
  variables:
    REGISTRY_IMAGE: registry.gitlab.com/<nama-anda>/<nama-repo>
    APP_NAME: "{app}"
    MENGPLOY_URL: $MENGPLOY_URL
    DEPLOY_TOKEN: $DEPLOY_TOKEN
  before_script:
    - docker login -u "$CI_REGISTRY_USER" -p "$CI_REGISTRY_PASSWORD" "$CI_REGISTRY"
  script:
    - docker build -t "$REGISTRY_IMAGE:$CI_COMMIT_SHA" .
    - docker push "$REGISTRY_IMAGE:$CI_COMMIT_SHA"
    - |
      DIGEST=$(docker inspect --format='{{{{index .RepoDigests 0}}}}' "$REGISTRY_IMAGE:$CI_COMMIT_SHA")
      curl -fsS -X POST "$MENGPLOY_URL/api/v1/deploy" \
        -H "Authorization: Bearer $DEPLOY_TOKEN" \
        -H "Content-Type: application/json" \
        -d "{{\"app\":\"$APP_NAME\",\"image\":\"$DIGEST\",\"commit\":\"$CI_COMMIT_SHA\",\"ref\":\"$CI_COMMIT_REF_NAME\"}}"
  only:
    - main
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_workflow_memuat_nama_app_dan_endpoint_deploy() {
        let workflow = github_actions_workflow("api-gateway");
        assert!(workflow.contains("api-gateway"));
        assert!(workflow.contains("POST \"$MENGPLOY_URL/api/v1/deploy\""));
        assert!(workflow.contains("DEPLOY_TOKEN"));
        // Nama file yang disarankan untuk workflow GitHub Actions.
        assert!(GITHUB_ACTIONS_PATH.starts_with(".github/workflows/"));
    }

    #[test]
    fn gitlab_workflow_memuat_nama_app_dan_endpoint_deploy() {
        let workflow = gitlab_ci_workflow("api-gateway");
        assert!(workflow.contains("api-gateway"));
        assert!(workflow.contains("POST \"$MENGPLOY_URL/api/v1/deploy\""));
        assert!(workflow.contains("DEPLOY_TOKEN"));
    }

    #[test]
    fn nama_app_dengan_kutip_di_escape_untuk_yaml() {
        let workflow = github_actions_workflow("app\"jahat");
        assert!(workflow.contains(r#"app\"jahat"#));
        assert!(!workflow.contains("app\"jahat"));
    }

    #[test]
    fn nama_app_dengan_backslash_di_escape_untuk_yaml() {
        let workflow = github_actions_workflow("app\\jahat");
        assert!(workflow.contains(r#"app\\jahat"#));
        assert!(!workflow.contains("app\\jahat"));
    }

    #[test]
    fn kedua_workflow_memakai_digest_bukan_tag() {
        assert!(github_actions_workflow("a").contains(".RepoDigests"));
        assert!(gitlab_ci_workflow("a").contains(".RepoDigests"));
    }
}
