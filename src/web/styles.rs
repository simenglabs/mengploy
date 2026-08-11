//! Token visual dan gaya global aplikasi.
//!
//! Sumber kebenaran tunggal untuk warna, tipografi, dan spasi — sesuai
//! `docs/design/login.md` §2 dan `docs/design/shell-aplikasi.md` §2. Tidak
//! ada file CSS terpisah, tidak ada Tailwind, tidak ada npm
//! (`docs/prd.md` §1.6). Disisipkan inline lewat tag `<style>` di layout.

/// CSS global aplikasi. Berisi token custom property, reset dasar, dan kelas
/// utilitas yang dipakai layout login serta shell dashboard/error.
pub const CSS: &str = r#"
:root {
  --color-bg-page: #111;
  /* Kontras kedalaman ekstra untuk area konsol log (`docs/design/log-viewer.md`
     §2): meniru terminal fisik supaya mata tidak cepat lelah saat debugging. */
  --color-bg-log: #070707;
  --color-bg-input: #1a1a1a;
  --color-bg-btn: #2a2a2a;
  --color-bg-btn-hover: #333;
  --color-text-main: #ddd;
  --color-text-muted: #888;
  --color-border: #444;
  --color-link: #6cf;
  --color-success: #6c6;
  --color-warning: #fc3;
  --color-danger: #f55;
  --font-mono: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
  --page-padding: 2rem;
  --max-form-width: 32rem;
}

* {
  box-sizing: border-box;
}

body {
  margin: 0;
  background-color: var(--color-bg-page);
  color: var(--color-text-main);
  font: var(--font-mono);
}

a {
  color: var(--color-link);
}

a:focus,
input:focus,
button:focus {
  outline: 2px solid var(--color-link);
  outline-offset: 2px;
}

/* --- Halaman login --- */

.login-page {
  display: grid;
  place-items: center;
  min-height: 100vh;
  padding: var(--page-padding);
}

.login-container {
  width: 100%;
  max-width: var(--max-form-width);
}

.login-logo {
  text-align: center;
  margin-bottom: 1.5rem;
  letter-spacing: 0.1em;
  color: var(--color-text-main);
}

.login-card {
  border: 1px solid var(--color-border);
  padding: 2rem;
}

.login-card h1 {
  margin-top: 0;
  font-size: 1.1rem;
}

.field {
  margin-bottom: 1.25rem;
}

.field label {
  display: block;
  margin-bottom: 0.4rem;
  color: var(--color-text-muted);
}

.field input[type="password"] {
  width: 100%;
  background-color: var(--color-bg-input);
  border: 1px solid var(--color-border);
  color: var(--color-text-main);
  padding: 0.5rem;
  font: var(--font-mono);
}

.field input.field-error {
  border-color: var(--color-danger);
}

.field-hint {
  color: var(--color-text-muted);
  margin: 0.4rem 0 0;
  font-size: 0.9em;
}

.btn {
  background-color: var(--color-bg-btn);
  color: var(--color-text-main);
  border: 1px solid var(--color-border);
  padding: 0.5rem 1rem;
  font: var(--font-mono);
  cursor: pointer;
}

.btn:hover {
  background-color: var(--color-bg-btn-hover);
}

.alert {
  padding: 0.75rem 1rem;
  margin-bottom: 1rem;
  border: 1px solid var(--color-border);
}

.alert-danger {
  color: var(--color-danger);
  border-color: var(--color-danger);
}

.alert-warning {
  color: var(--color-warning);
  border-color: var(--color-warning);
}

@media (max-width: 36rem) {
  .login-page {
    padding: 1rem;
  }
}

/* --- Shell aplikasi (dashboard & error) --- */

.app-layout {
  display: grid;
  grid-template-columns: 16rem 1fr;
  min-height: 100vh;
}

.sidebar {
  border-right: 1px solid var(--color-border);
  padding: 1rem;
}

.sidebar .brand {
  margin-bottom: 1.5rem;
  letter-spacing: 0.05em;
}

.sidebar .brand .phase-tag {
  color: var(--color-text-muted);
  font-size: 0.85em;
}

.sidebar nav ul {
  list-style: none;
  margin: 0;
  padding: 0;
}

.sidebar nav a {
  display: block;
  padding: 0.5rem 0;
  color: var(--color-text-main);
  text-decoration: none;
}

.sidebar nav a[aria-current="page"] {
  color: var(--color-link);
}

.main-column {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.app-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 1rem var(--page-padding);
  border-bottom: 1px solid var(--color-border);
}

.status-active {
  color: var(--color-success);
}

.form-logout {
  margin: 0;
}

.app-content {
  padding: var(--page-padding);
  word-break: break-word;
}

.card-placeholder {
  border: 1px solid var(--color-border);
  padding: 1.5rem;
}

.card-placeholder h2 {
  margin-top: 0;
}

.error-box {
  border: 1px solid var(--color-border);
  padding: 1.5rem;
}

.error-box.warning {
  border-color: var(--color-warning);
  color: var(--color-warning);
}

.error-box.danger {
  border-color: var(--color-danger);
  color: var(--color-danger);
}

@media (max-width: 48rem) {
  .app-layout {
    display: flex;
    flex-direction: column;
  }

  .sidebar {
    border-right: none;
    border-bottom: 1px solid var(--color-border);
  }

  .app-content {
    padding: 1rem;
  }
}

/* --- Fase 1: header/strip --- */

.app-header {
  flex-wrap: wrap;
  gap: 0.75rem;
}

.fleet-strip-slot {
  flex: 1;
  min-width: 0;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 1rem;
}

.fleet-strip {
  display: flex;
  flex-wrap: wrap;
  gap: 1rem;
  list-style: none;
  margin: 0;
  padding: 0;
}

.fleet-strip a {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  min-height: 44px;
  text-decoration: none;
  color: var(--color-text-main);
}

.fleet-strip-name {
  max-width: 10rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fleet-strip-empty {
  color: var(--color-text-muted);
}

.status-dot {
  display: inline-block;
  width: 0.6rem;
  height: 0.6rem;
  border-radius: 50%;
  background-color: var(--color-text-muted);
}

.status-dot.online { background-color: var(--color-success); }
.status-dot.unreachable { background-color: var(--color-danger); }
.status-dot.pending { background-color: var(--color-text-muted); }
.status-dot.verifying {
  background-color: var(--color-warning);
  animation: pulse 1.5s infinite;
}

@keyframes pulse {
  0% { opacity: 0.6; }
  50% { opacity: 1; }
  100% { opacity: 0.6; }
}

@media (max-width: 48rem) {
  .fleet-strip {
    flex-wrap: wrap;
  }
}

/* --- Fleet overview (/servers) --- */

.fleet-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

.fleet-empty {
  border: 1px dashed var(--color-border);
  padding: 2rem;
  text-align: center;
}

.fleet-table {
  width: 100%;
  border-collapse: collapse;
  margin-top: 1.5rem;
}

.fleet-table th,
.fleet-table td {
  border-bottom: 1px solid var(--color-border);
  padding: 0.75rem 1rem;
  text-align: left;
}

.fleet-table tbody tr.unreachable-row {
  background-color: rgba(255, 85, 85, 0.05);
}

.fleet-table a.name-danger {
  color: var(--color-danger);
}

.row-detail {
  font-size: 0.85em;
  color: var(--color-text-muted);
}

.row-detail.warning {
  color: var(--color-warning);
}

.status-badge {
  display: inline-block;
  padding: 0.2rem 0.5rem;
  font-size: 0.85em;
  border: 1px solid currentColor;
}

.status-badge.online { color: var(--color-success); }
.status-badge.unreachable { color: var(--color-danger); }
.status-badge.pending { color: var(--color-text-muted); }
.status-badge.verifying {
  color: var(--color-warning);
  animation: pulse 1.5s infinite;
}

@media (max-width: 48rem) {
  .fleet-table th:nth-child(5),
  .fleet-table td:nth-child(5),
  .fleet-table th:nth-child(6),
  .fleet-table td:nth-child(6) {
    display: none;
  }
}

/* --- Wizard tambah server --- */

.verify-checklist {
  list-style: none;
  margin: 1.5rem 0;
  padding: 0;
}

.verify-step {
  border: 1px solid var(--color-border);
  padding: 0.75rem 1rem;
  margin-bottom: 0.5rem;
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.5rem;
}

.verify-step-symbol {
  font-weight: bold;
}

.verify-step.todo .verify-step-symbol { color: var(--color-text-muted); }
.verify-step.running {
  border-color: var(--color-warning);
}
.verify-step.running .verify-step-symbol {
  color: var(--color-warning);
  animation: pulse 1.5s infinite;
}
.verify-step.success {
  border-color: var(--color-success);
}
.verify-step.success .verify-step-symbol { color: var(--color-success); }
.verify-step.danger {
  border-color: var(--color-danger);
}
.verify-step.danger .verify-step-symbol { color: var(--color-danger); }

.verify-step-message {
  flex-basis: 100%;
  margin: 0.25rem 0 0;
  color: var(--color-danger);
}

.tofu-box {
  border: 2px solid var(--color-warning);
  padding: 1rem;
  margin-bottom: 1rem;
}

.tofu-box .host-key {
  display: inline-block;
  margin: 0.5rem 0;
}

textarea,
input[type="text"],
input[type="number"],
input[type="password"] {
  width: 100%;
  background-color: var(--color-bg-input);
  border: 1px solid var(--color-border);
  color: var(--color-text-main);
  padding: 0.6rem 0.75rem;
  font: var(--font-mono);
  border-radius: 2px;
  transition: border-color 0.15s ease;
}

textarea:hover,
input[type="text"]:hover,
input[type="number"]:hover,
input[type="password"]:hover {
  border-color: #666;
}

textarea::placeholder,
input::placeholder {
  color: var(--color-text-muted);
}

textarea {
  line-height: 1.5;
}

/* Panel form (wizard tambah server, registry) — kotak terkontain supaya
   tidak melebar penuh viewport, konsisten dengan `.login-card`/`.detail-card`. */
.form-panel {
  max-width: 40rem;
  border: 1px solid var(--color-border);
  padding: 1.75rem 2rem;
  margin-top: 1.5rem;
}

/* Label kiri, input kanan di layar lebar — persis `docs/design/tambah-server.md`
   §3 ("Label di sebelah kiri, kolom input di sebelah kanan"). Di bawah
   48rem tetap ditumpuk (default block `.field` di atas). */
@media (min-width: 48rem) {
  .form-panel .field {
    display: grid;
    grid-template-columns: 10rem 1fr;
    column-gap: 1.5rem;
    align-items: start;
  }

  .form-panel .field label {
    padding-top: 0.6rem;
    margin-bottom: 0;
  }

  .form-panel .field-hint {
    grid-column: 2;
  }
}

#port {
  max-width: 8rem;
}

.field-radio {
  margin-bottom: 0.5rem;
}

.field-radio label {
  margin-left: 0.4rem;
  color: var(--color-text-main);
}

.field-actions {
  display: flex;
  gap: 1rem;
  align-items: center;
  margin-top: 1.75rem;
}

/* Input readonly berisi perintah siap-salin (mis. `ssh-keygen ...`). Klik
   menyalin ke clipboard, `.copy-tooltip` muncul sebentar sebagai konfirmasi. */
.copy-field {
  position: relative;
}

.copy-field input[readonly] {
  cursor: pointer;
  color: var(--color-link);
}

.copy-field input[readonly]:hover {
  border-color: var(--color-link);
}

.copy-tooltip {
  position: absolute;
  top: -1.9rem;
  left: 0;
  background-color: var(--color-bg-btn);
  border: 1px solid var(--color-success);
  color: var(--color-success);
  padding: 0.2rem 0.5rem;
  font-size: 0.85em;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.15s ease;
}

.copy-tooltip.show {
  opacity: 1;
}

code {
  background-color: var(--color-bg-page);
  padding: 0.1rem 0.3rem;
  font: var(--font-mono);
}

.btn-secondary {
  background-color: transparent;
}

a.btn {
  display: inline-block;
  text-decoration: none;
}

/* --- Detail server --- */

.detail-title-row {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1rem;
}

.detail-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1.5rem;
  margin-bottom: 1.5rem;
}

.detail-card {
  background-color: var(--color-bg-input);
  border: 1px solid var(--color-border);
  padding: 1.5rem;
  margin-bottom: 1.5rem;
}

.detail-card h2 {
  margin-top: 0;
  font-size: 1.1rem;
  border-bottom: 1px solid var(--color-border);
  padding-bottom: 0.5rem;
  margin-bottom: 1rem;
}

.detail-row {
  display: flex;
  justify-content: space-between;
  margin-bottom: 0.75rem;
  gap: 1rem;
}

.detail-row span:first-child {
  color: var(--color-text-muted);
}

.host-key {
  background-color: var(--color-bg-page);
  padding: 0.2rem 0.4rem;
  font-size: 0.9em;
  word-break: break-all;
}

.metric-placeholder p {
  color: var(--color-text-muted);
}

/* --- Metrik Fase 6 --- */
.metrics-panel {
  overflow: hidden;
}
.metrics-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 1rem;
  border-bottom: 1px solid var(--color-border);
  padding-bottom: 0.75rem;
}
.metrics-header h2 {
  border: 0;
  padding: 0;
  margin: 0;
}
.metrics-caption,
.chart-legend {
  color: var(--color-text-muted);
  font-size: 0.85em;
}
.metrics-range {
  display: flex;
  gap: 0.35rem;
  flex-wrap: wrap;
}
.metrics-range a {
  border: 1px solid var(--color-border);
  padding: 0.35rem 0.55rem;
  text-decoration: none;
  color: var(--color-text-muted);
}
.metrics-range a:hover,
.metrics-range a.range-active {
  color: var(--color-text-main);
  border-color: var(--color-link);
}
.metrics-empty {
  border: 1px dashed var(--color-border);
  padding: 1.25rem;
  color: var(--color-text-muted);
}
.metric-cards {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 0.75rem;
  margin: 1rem 0;
}
.metric-card {
  border: 1px solid var(--color-border);
  padding: 0.9rem;
  min-width: 0;
}
.metric-card-label {
  display: block;
  color: var(--color-text-muted);
  font-size: 0.85em;
}
.metric-card-value {
  display: block;
  margin: 0.25rem 0 0.5rem;
}
.sparkline {
  display: block;
  letter-spacing: 0.05em;
  color: var(--color-success);
  overflow: hidden;
  white-space: nowrap;
}
.metric-chart {
  border-top: 1px solid var(--color-border);
  padding-top: 0.75rem;
  margin-top: 1rem;
}
.metric-chart h3,
.container-chart h4,
.alert-panel h3 {
  margin-bottom: 0.35rem;
}
.chart-wrap {
  border: 1px solid var(--color-border);
  background: var(--color-bg-log);
  padding: 0.5rem;
}
.chart-svg {
  display: block;
  width: 100%;
  height: 12rem;
}
.chart-axis {
  stroke: var(--color-border);
  stroke-width: 0.5;
}
.chart-line {
  fill: none;
  stroke-width: 1.5;
  vector-effect: non-scaling-stroke;
}
.chart-cpu { stroke: var(--color-success); }
.chart-memory { stroke: var(--color-warning); }
.chart-disk { stroke: var(--color-link); }
.chart-wrap .chart-legend { margin: 0.35rem 0 0; }
.chart-deployment {
  stroke: var(--color-danger);
  stroke-dasharray: 2 2;
  stroke-width: 0.75;
  vector-effect: non-scaling-stroke;
}
.chart-data {
  margin: 0.5rem 0 0;
  padding: 0.5rem;
  overflow-x: auto;
  min-height: 2.5rem;
  color: var(--color-text-main);
}
.deployment-markers {
  border-left: 3px solid var(--color-link);
  padding: 0.6rem 0.75rem;
  color: var(--color-link);
  overflow-wrap: anywhere;
}
.alert-panel {
  border-top: 1px solid var(--color-border);
  margin-top: 1rem;
  padding-top: 0.75rem;
}
.alert-list {
  list-style: none;
  padding: 0;
  margin: 0;
}
.alert-item {
  border-left: 3px solid var(--color-warning);
  padding: 0.55rem 0.75rem;
  margin-bottom: 0.5rem;
}
.alert-item.alert-critical {
  border-left-color: var(--color-danger);
  color: var(--color-danger);
}
@media (max-width: 48rem) {
  .metrics-header {
    flex-direction: column;
  }
  .metric-cards {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 48rem) {
  .detail-grid {
    grid-template-columns: 1fr;
  }
}

/* ── Viewer log Fase 3 (`docs/design/log-viewer.md`) ────────────────────── */
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
.log-back {
  margin-bottom: 0.5rem;
}
.log-privacy-note {
  color: var(--color-text-muted);
  font-size: 0.85rem;
  border: 1px solid var(--color-border);
  background-color: var(--color-bg-input);
  padding: 0.5rem 0.75rem;
  margin: 0 0 0.75rem 0;
}
.log-toolbar {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  background-color: var(--color-bg-input);
  border: 1px solid var(--color-border);
  border-bottom: 0;
  padding: 0.5rem 0.75rem;
}
.log-search,
.log-toggles {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}
.log-search input[type="search"] {
  flex: 1 1 12rem;
  min-width: 0;
}
.log-toggle {
  color: var(--color-text-muted);
  white-space: nowrap;
}
.log-download-disabled {
  color: var(--color-text-muted);
  border: 1px dashed var(--color-border);
  padding: 0.3rem 0.6rem;
  cursor: not-allowed;
}
.log-console-shell {
  position: relative;
}
.log-status-row {
  background-color: var(--color-bg-input);
  border: 1px solid var(--color-border);
  border-bottom: 0;
  padding: 0.3rem 0.75rem;
  font-size: 0.8rem;
  letter-spacing: 0.05em;
}
.log-status-arsip {
  color: var(--color-text-muted);
}
/* Sunyi BUKAN putus: selama SSE terbuka, indikator tetap hijau walau tidak ada
   baris baru (`docs/design/log-viewer.md` §4.2). Label putus baru tampil kalau
   JS menambahkan `log-status-terputus` dari event `htmx:sseError`. */
.log-status-streaming .log-status-sehat {
  color: var(--color-success);
}
.log-status-streaming .log-status-putus,
.log-status-streaming .log-status-putus-detail {
  display: none;
  color: var(--color-warning);
}
.log-status-streaming.log-status-terputus .log-status-sehat {
  display: none;
}
.log-status-streaming.log-status-terputus .log-status-putus,
.log-status-streaming.log-status-terputus .log-status-putus-detail {
  display: inline;
}
.log-status-streaming.log-status-terputus .log-status-putus-detail {
  margin-left: 0.5rem;
}
.log-console {
  background-color: var(--color-bg-log);
  border: 1px solid var(--color-border);
  color: var(--color-text-main);
  font: var(--font-mono);
  height: 60vh;
  min-height: 400px;
  margin: 0;
  padding: 0.5rem 0;
  overflow-y: auto;
  overflow-x: auto;
  white-space: pre;
}
.log-console-wrap {
  white-space: pre-wrap;
  word-break: break-all;
  overflow-x: hidden;
}
.log-line {
  display: flex;
  gap: 0.75rem;
  padding: 0 1.5rem;
}
.log-console-wrap .log-line {
  flex-wrap: wrap;
}
.log-gutter {
  flex: 0 0 4.5rem;
  color: var(--color-text-muted);
  text-align: right;
  user-select: none;
}
.log-text {
  flex: 1 1 auto;
  min-width: 0;
}
.log-line-info .log-text {
  color: var(--color-text-muted);
}
.log-line-warning {
  background-color: var(--color-bg-input);
  border-left: 3px solid var(--color-warning);
}
.log-line-warning .log-text {
  color: var(--color-warning);
}
.log-line-danger {
  background-color: var(--color-bg-input);
  border-left: 3px solid var(--color-danger);
}
.log-line-danger .log-text {
  color: var(--color-danger);
}
.log-back-to-bottom {
  position: absolute;
  right: 1.5rem;
  bottom: 1rem;
  background-color: var(--color-bg-btn);
  color: var(--color-text-main);
  border: 1px solid var(--color-border);
  padding: 0.3rem 0.6rem;
  cursor: pointer;
}
.log-back-to-bottom:hover {
  background-color: var(--color-bg-btn-hover);
}
.app-tabs ul {
  display: flex;
  gap: 0.25rem;
  list-style: none;
  margin: 0 0 1rem 0;
  padding: 0;
  border-bottom: 1px solid var(--color-border);
}
.app-tab {
  display: inline-block;
  padding: 0.4rem 0.9rem;
  border: 1px solid var(--color-border);
  border-bottom: 0;
  color: var(--color-text-muted);
  text-decoration: none;
}
.app-tab-aktif {
  background-color: var(--color-bg-input);
  color: var(--color-text-main);
}
.digest-cell {
  word-break: break-all;
  font-size: 0.85em;
}
@media (min-width: 48rem) {
  .log-toolbar {
    flex-direction: row;
    justify-content: space-between;
    align-items: center;
  }
}
@media (max-width: 48rem) {
  .log-console {
    font-size: 12px;
  }
  .log-line {
    padding: 0 0.75rem;
  }
  /* Hemat ruang horizontal di layar kecil: gutter disembunyikan supaya isi log
     tidak terpotong (`docs/design/log-viewer.md` §6). */
  .log-gutter {
    display: none;
  }
}
.fleet-target-list {
  display: grid;
  gap: 0.45rem;
  margin: 1rem 0;
}
.fleet-target-list label,
.fleet-exec-form label {
  display: block;
  color: var(--color-text-main);
}
.fleet-output {
  max-height: 24rem;
  overflow: auto;
  white-space: pre-wrap;
  background: var(--color-bg-log);
  border: 1px solid var(--color-border);
  padding: 1rem;
}
.fleet-exec-form {
  display: grid;
  grid-template-columns: 1fr 1fr auto auto;
  gap: 0.5rem;
  align-items: center;
  margin-top: 0.75rem;
}
@media (max-width: 48rem) {
  .fleet-exec-form { grid-template-columns: 1fr; }
}
"#;
