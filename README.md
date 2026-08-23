# Deep

Deep adalah **agent harness untuk riset berbasis bukti** yang ditulis dengan Rust. Produk ini tidak dirancang sebagai chatbot umum. Tujuan utamanya adalah melakukan penelusuran web, membaca sumber asli, mencatat bukti, menghubungkan bukti ke klaim, menguji kontradiksi, lalu menghasilkan kesimpulan yang tidak lebih kuat daripada bukti yang tersedia.

> Evidence first. Conclusions second.

## Status

Repositori ini masih berada pada tahap MVP. Fondasi yang sudah tersedia meliputi:

- TUI berbasis `ratatui` dan `crossterm`;
- agent loop asinkron berbasis `tokio`;
- LLM melalui endpoint OpenAI-compatible `chat/completions`;
- Firecrawl API v2 untuk `search`, `scrape`, `map`, `crawl`, status crawl, dan `interact`;
- research workspace berbasis SQLite;
- claim → evidence linkage;
- deteksi sumber dengan konten identik melalui SHA-256;
- aturan bahwa hasil pencarian hanya dipakai untuk discovery, bukan sebagai evidence;
- status epistemik klaim yang disimpan secara terstruktur;
- credential profiles yang dapat ditumpuk dan dipilih langsung dari TUI;
- CI untuk `rustfmt`, `clippy`, dan test.

## Filosofi inti

Deep memisahkan tiga hal yang sering tercampur pada agent riset biasa:

```text
LLM investigator
      ↓
research workspace
      ↓
epistemic constraints
```

LLM boleh memilih jalur investigasi, menghasilkan query, memilih sumber, dan mengusulkan perubahan status klaim. Namun harness tetap membatasi perubahan state tertentu. Sebagai contoh, klaim tidak dapat menjadi `VERIFIED`, `SUPPORTED`, atau `DISPROVEN` sebelum mempunyai evidence yang benar-benar terhubung.

Hasil `search` juga tidak dapat langsung dimasukkan sebagai evidence. URL harus dibaca terlebih dahulu melalui `scrape`, baru kutipan dari sumber tersebut dapat dicatat.

## Menjalankan

Persyaratan utama adalah toolchain Rust stabil.

```bash
cargo run
```

Tanpa subcommand, Deep langsung membuka TUI. Bentuk eksplisitnya:

```bash
cargo run -- tui
```

Untuk melihat lokasi workspace dan berkas credential tanpa menampilkan isinya:

```bash
cargo run -- paths
```

## Login dan credential stack

Jika belum terdapat credential aktif, Deep langsung membuka layar **Credential Stack**.

Di layar tersebut:

```text
l       tambah profil LLM
f       tambah profil Firecrawl
↑ / ↓   pilih profil
Enter   jadikan profil aktif
h/Esc   kembali
q       keluar
```

Setiap profil LLM menyimpan:

```text
profile name
base URL
model
API key
```

Karena client LLM hanya bergantung pada antarmuka OpenAI-compatible, base URL dapat diarahkan ke provider lain yang menyediakan kontrak `chat/completions` yang kompatibel.

Setiap profil Firecrawl menyimpan:

```text
profile name
base URL
API key
```

Beberapa profil dapat disimpan sekaligus. Satu profil LLM dan satu profil Firecrawl dipilih sebagai profil aktif pada suatu waktu.

Sebagai fallback, Deep juga membaca environment variable berikut apabila tidak ada profil tersimpan yang dapat digunakan:

```text
OPENAI_API_KEY
OPENAI_BASE_URL
OPENAI_MODEL
FIRECRAWL_API_KEY
FIRECRAWL_BASE_URL
```

## Penyimpanan credential

Credential disimpan di direktori konfigurasi aplikasi milik sistem operasi. Pada sistem Unix, Deep mengatur permission berkas credential menjadi `0600`.

Implementasi MVP saat ini masih menyimpan credential dalam berkas JSON lokal. Artinya proteksinya bergantung pada permission akun sistem operasi dan belum menggunakan macOS Keychain, Windows Credential Manager, atau secret service lain. Ini merupakan keterbatasan yang disengaja agar fondasi MVP tetap sederhana dan dapat diuji lintas platform.

## TUI

Layar investigasi utama dibagi menjadi beberapa state nyata, bukan pseudo-progress.

```text
┌ CASE / OBJECTIVE ────────────────────────────────────────────┐
├ ACTIVITY ─────────────────────┬ CLAIMS ──────────────────────┤
│ SEARCH                       │ C1 VERIFIED                  │
│ SCRAPE                       │ C2 UNRESOLVED                │
│ EVIDENCE                     ├ SOURCES ─────────────────────┤
│ VERIFY                       │ S1 ...                       │
│ FOLLOW                       │ S2 ...                       │
└──────────────────────────────┴───────────────────────────────┘
```

Warna dipakai secara semantik:

- cyan untuk operasi retrieval;
- hijau untuk evidence dan verifikasi;
- kuning untuk claim, lead, dan state yang belum final;
- merah untuk error, rejection, atau kegagalan;
- ungu untuk sumber/entitas;
- abu-abu untuk sumber duplikat.

TUI tidak menampilkan raw chain-of-thought model. Yang ditampilkan hanyalah event operasional yang benar-benar terjadi pada harness.

## Research workspace

SQLite menyimpan state utama berikut:

```text
cases
sources
claims
evidence
claim_evidence
events
```

Source mempunyai `content_hash`. Jika dua URL menghasilkan konten yang sama, source berikutnya ditandai melalui `duplicate_of` sehingga URL tambahan tidak otomatis dianggap sebagai corroboration independen.

Evidence hanya dapat dibuat jika `source_id` berasal dari source yang telah masuk ke case yang sama.

## Status klaim

Status yang saat ini didukung:

```text
VERIFIED
SUPPORTED
UNRESOLVED
CONFLICTING
DISPROVEN
INSUFFICIENT_EVIDENCE
DEAD_END
```

`VERIFIED`, `SUPPORTED`, dan `DISPROVEN` membutuhkan minimal satu evidence yang sudah dihubungkan ke klaim.

## Tool agent

Tool yang diberikan kepada investigator saat ini:

```text
search
scrape
map
crawl
crawl_status
interact
calculator
date_days_between
record_claim
record_evidence
link_evidence
set_claim_status
finish_report
reject_request
```

Agent tidak mempunyai arbitrary shell, package manager, filesystem bebas, coding tools, atau multi-agent swarm.

## Stop condition

Model diarahkan untuk berhenti ketika salah satu kondisi epistemik tercapai, antara lain evidence sudah cukup, evidence tetap bertentangan, sumber yang dapat dipercaya tidak tersedia, lead telah habis, atau pencarian baru mulai mengalami diminishing return.

Selain itu terdapat `MAX_AGENT_STEPS` sebagai guardrail operasional. Guardrail ini bukan definisi bahwa investigasi sudah selesai secara epistemik. Jika guardrail tercapai lebih dahulu, laporan menyatakan bahwa penghentian tersebut bersifat operasional.

## Struktur proyek

```text
src/
├── agent.rs           # investigator loop dan tool contract
├── clients.rs         # OpenAI-compatible + Firecrawl
├── credentials.rs     # credential stack dan profile selection
├── store.rs           # SQLite research workspace
├── main.rs
└── tui/
    ├── mod.rs         # terminal runtime
    ├── app.rs         # state dan input handling
    └── render.rs      # renderer investigation console

migrations/
└── 0001_init.sql
```

## Pengujian

Secara lokal:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Perintah yang sama dijalankan oleh GitHub Actions.

## Batas MVP saat ini

Beberapa bagian PRD belum diimplementasikan penuh, khususnya entity graph, timeline engine, provenance graph lintas sumber, relationship objects, structured table operations, archive retrieval, DNS/RDAP, export investigation bundle, dan evaluator benchmark. Komponen tersebut sebaiknya ditambahkan setelah fondasi claim/evidence dan agent loop terbukti stabil.
