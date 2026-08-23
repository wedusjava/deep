# Deep

Deep adalah **agent harness untuk riset berbasis bukti** yang ditulis dengan Rust. Produk ini tidak dirancang sebagai chatbot umum. Tujuan utamanya adalah melakukan penelusuran web, membaca sumber asli, mencatat bukti, memisahkan klaim dari dugaan, menghubungkan entitas dan relasi, mengejar lead, menguji kontradiksi, lalu menghasilkan kesimpulan yang tidak lebih kuat daripada bukti yang tersedia.

> Evidence first. Conclusions second.

## Status

Repositori ini berada pada tahap MVP. Fondasi yang sudah tersedia meliputi:

- TUI berbasis `ratatui` dan `crossterm`;
- agent loop asinkron berbasis `tokio`;
- LLM melalui endpoint OpenAI-compatible `chat/completions`;
- Firecrawl API v2 untuk `search`, `scrape`, `map`, `crawl`, status crawl, dan `interact`;
- research workspace berbasis SQLite;
- claim → evidence linkage;
- relationship → evidence linkage;
- entitas, relationships, leads, dan notes sebagai objek workspace terstruktur;
- deteksi sumber dengan konten identik melalui SHA-256;
- aturan bahwa hasil pencarian hanya dipakai untuk discovery, bukan sebagai evidence;
- status epistemik klaim dan relasi yang divalidasi oleh harness;
- calculator, date math, statistik dasar, dan text diff yang deterministik;
- credential profiles yang dapat ditumpuk dan dipilih langsung dari TUI;
- CI untuk `rustfmt`, `clippy`, dan test.

## Filosofi inti

Deep memisahkan komponen yang sering tercampur pada agent riset biasa:

```text
LLM investigator
      ↓
research actions
      ↓
structured workspace
      ↓
epistemic constraints
      ↓
report
```

LLM boleh memilih jalur investigasi, menghasilkan query, memilih sumber, membuat claim, entity, relationship, atau lead, serta mengusulkan perubahan status. Namun harness tetap mengendalikan transisi epistemik tertentu.

Sebagai contoh:

- `VERIFIED` dan `SUPPORTED` membutuhkan evidence yang mendukung;
- `DISPROVEN` membutuhkan evidence yang membantah;
- `CONFLICTING` membutuhkan evidence pada kedua sisi;
- aturan yang sama berlaku pada relationship;
- hasil `search` tidak dapat langsung dijadikan evidence;
- notes tidak pernah dianggap evidence dengan sendirinya.

URL harus dibaca melalui `scrape` sebelum kutipan dari halaman tersebut dapat masuk ke evidence ledger.

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

Layar investigasi utama menampilkan state nyata, bukan pseudo-progress.

```text
┌ CASE / OBJECTIVE ────────────────────────────────────────────┐
├ ACTIVITY ─────────────────────┬ CLAIMS ──────────────────────┤
│ SEARCH                       │ C1 VERIFIED                  │
│ SCRAPE                       │ C2 UNRESOLVED                │
│ EVIDENCE                     ├ LEADS ───────────────────────┤
│ ENTITY                       │ L1 ACTIVE                    │
│ RELATIONSHIP                 ├ SOURCES ─────────────────────┤
│ FOLLOW                       │ S1 PRIMARY                   │
└──────────────────────────────┴───────────────────────────────┘
```

Warna dipakai secara semantik:

- cyan untuk retrieval dan lead aktif;
- hijau untuk evidence dan verifikasi;
- kuning untuk claim atau lead yang belum final;
- merah untuk contradiction, dead end, rejection, atau error;
- ungu untuk entity dan relationship discovery;
- abu-abu untuk sumber duplikat atau lead yang dibuang.

TUI tidak menampilkan raw chain-of-thought model. Yang ditampilkan hanyalah event operasional yang benar-benar terjadi pada harness.

## Research workspace

SQLite menyimpan state utama berikut:

```text
cases
sources
claims
evidence
claim_evidence
entities
relationships
relationship_evidence
leads
notes
events
```

Source mempunyai `content_hash`. Jika dua URL menghasilkan konten yang sama, source berikutnya ditandai melalui `duplicate_of` sehingga URL tambahan tidak otomatis dianggap sebagai corroboration independen.

Evidence hanya dapat dibuat jika source berasal dari case yang sama. Claim dan relationship juga hanya dapat ditautkan ke evidence dalam case yang sama.

Entity dapat dihubungkan melalui relationship terarah. Relationship mulai dari `UNRESOLVED` dan tunduk pada constraint evidence yang sama seperti claim.

Lead mempunyai state:

```text
OPEN
ACTIVE
EXHAUSTED
DISCARDED
```

Dengan demikian agent dapat membedakan lead yang masih produktif, sedang dikerjakan, sudah mencapai jalan buntu, atau sengaja dibuang.

## Status epistemik

Status claim dan relationship yang saat ini didukung:

```text
VERIFIED
SUPPORTED
UNRESOLVED
CONFLICTING
DISPROVEN
INSUFFICIENT_EVIDENCE
DEAD_END
```

Harness tidak memakai angka confidence semu sebagai pengganti evidence trail.

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
statistics
text_diff

record_claim
record_evidence
link_evidence
set_claim_status

record_entity
record_relationship
link_relationship_evidence
set_relationship_status

record_lead
set_lead_status
record_note

finish_report
reject_request
```

`statistics` menyediakan operasi:

```text
sum
mean
median
min
max
percentage_change
standard_deviation
percentile
correlation
```

`text_diff` membandingkan dua versi teks secara line-based dan deterministik.

Agent tidak mempunyai arbitrary shell, package manager, filesystem bebas, coding tools, atau multi-agent swarm.

## Stop condition

Model diarahkan untuk berhenti ketika salah satu kondisi epistemik tercapai, antara lain evidence sudah cukup, evidence tetap bertentangan, sumber yang dapat dipercaya tidak tersedia, lead telah habis, atau pencarian baru mulai mengalami diminishing return.

Selain itu terdapat `MAX_AGENT_STEPS` sebagai guardrail operasional. Guardrail ini bukan definisi bahwa investigasi sudah selesai secara epistemik. Jika guardrail tercapai lebih dahulu, laporan menyatakan bahwa penghentian tersebut bersifat operasional.

## Laporan akhir

Report dibangun kembali dari state workspace, bukan hanya dari teks bebas model. MVP saat ini menampilkan:

```text
RESULT
CLAIMS
ENTITIES
RELATIONSHIPS
LEADS
SOURCES
LIMITATIONS
CONCLUSION
```

Claim dan relationship mencantumkan jumlah evidence yang terhubung. Source duplikat tetap terlihat beserta origin record yang identik.

## Struktur proyek

```text
src/
├── agent.rs           # investigator loop dan tool contract
├── analysis_tools.rs  # statistik dan text diff deterministik
├── clients.rs         # OpenAI-compatible + Firecrawl
├── credentials.rs     # credential stack dan profile selection
├── store.rs           # SQLite research workspace
├── main.rs
└── tui/
    ├── mod.rs         # terminal runtime
    ├── app.rs         # state dan input handling
    └── render.rs      # renderer investigation console

migrations/
├── 0001_init.sql
└── 0002_epistemic_objects.sql
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

Beberapa kemampuan yang lebih lanjut belum diimplementasikan penuh, khususnya visualisasi entity graph interaktif, timeline engine, provenance graph yang mampu mengenali derivative source meskipun kontennya tidak identik, structured table operations, archive retrieval, DNS/RDAP, certificate transparency, export investigation bundle, semantic retrieval/vector database, dan evaluator benchmark yang lengkap.

Komponen tersebut sebaiknya ditambahkan hanya setelah benchmark menunjukkan bahwa complexity tambahannya memberikan peningkatan kualitas riset yang dapat diukur.
