# ditdev_be_rust

Backend API untuk portofolio pribadi - ditulis ulang dari Node.js/Express ke **Rust**. Single binary, performa tinggi, dan type-safe.

## Fitur

- **Autentikasi** - JWT (login, register, verify, logout, daftar sesi aktif), bcrypt cost 12
- **Projects & Certificates** - CRUD lengkap dengan upload file ke Cloudflare R2
- **Stats** - statistik portofolio dengan auto-calc (months-diff, total projects)
- **Upload** - image (5 MB) dan PDF (10 MB) ke Cloudflare R2
- **Chat AI** - proxy ke Cerebras LLM dengan RAG context + rate limit per-IP
- **Contact form** - diteruskan ke Discord webhook
- **XP system** - XP harian deterministik (seeded PRNG) + bonus XP per visitor
- **GitHub activity** - proxy ke GitHub API dengan cache server-side
- **Health check** - liveness + status database & R2

## Tech Stack

| Concern | Teknologi |
|---|---|
| Web framework | [axum](https://github.com/tokio-rs/axum) |
| Runtime | tokio |
| Database | PostgreSQL via [sqlx](https://github.com/launchbadge/sqlx) (migration bawaan) |
| Object storage | Cloudflare R2 (`aws-sdk-s3`) |
| Auth | `jsonwebtoken`, `bcrypt` |
| HTTP client | `reqwest` |
| Middleware | `tower-http` (CORS, tracing) |
| Logging | `tracing` |

## Persyaratan

- Rust **stable ≥ 1.85** (edition 2024)
- PostgreSQL - bisa Neon (serverless) atau lokal

## Setup

```bash
# 1. Salin file environment lalu isi
cp .env.example .env

# 2. Jalankan
cargo run
```

Server akan berjalan di `http://localhost:2817`. Migrasi database **dijalankan otomatis** saat startup.

## Konfigurasi (Environment Variables)

| Variable | Wajib | Default | Keterangan |
|---|---|---|---|
| `PORT` | - | `2817` | Port server |
| `APP_ENV` | - | `development` | `production` menyembunyikan detail error |
| `LOG_LEVEL` | - | `info` | Level log |
| `DATABASE_URL` | ✅ | - | Connection string PostgreSQL |
| `DB_SSL_REJECT_UNAUTHORIZED` | - | `true` | Verifikasi TLS (nonaktifkan hanya untuk dev lokal) |
| `JWT_SECRET` | ✅ | - | Secret untuk sign/verify token |
| `JWT_EXPIRE` | - | `24h` | Masa berlaku token |
| `R2_ACCOUNT_ID` | ✅ | - | Cloudflare R2 |
| `R2_ACCESS_KEY_ID` | ✅ | - | Cloudflare R2 |
| `R2_SECRET_ACCESS_KEY` | ✅ | - | Cloudflare R2 |
| `R2_BUCKET_NAME` | ✅ | - | Nama bucket (`porto-ditdev`) |
| `R2_PUBLIC_URL` | ✅ | - | URL publik file (mis. `https://cdn.example.com`) |
| `CEREBRAS_API_KEY` | - | - | API key Cerebras (untuk chat AI) |
| `CEREBRAS_MODEL` | - | `gpt-oss-120b` | Model yang dipakai chat |
| `RAG_SERVICE_URL` | - | `http://localhost:8765` | Service RAG eksternal |
| `DISCORD_WEBHOOK_URL` | - | - | Webhook Discord (untuk contact form) |
| `GITHUB_USERNAME` | - | `rillToMe` | Username GitHub |
| `GITHUB_TOKEN` | - | - | Token GitHub (opsional, naikkan rate limit ke 5000/jam) |
| `CLIENT_URL` / `ADMIN_URL` | - | - | Whitelist CORS tambahan |

## API Endpoints

Semua respons mengikuti format `{ "success": boolean, "message"?: string, "data"?: ... }`.

### Auth - `/api/auth`
| Method | Path | Auth | Keterangan |
|---|---|---|---|
| POST | `/api/auth/login` | - | Login, dapatkan token |
| POST | `/api/auth/register` | JWT | Buat admin baru |
| GET | `/api/auth/verify` | JWT | Verifikasi token |
| POST | `/api/auth/logout` | JWT | Logout (token di-blacklist) |
| GET | `/api/auth/sessions` | JWT | Daftar sesi aktif |

### Content - `/api/projects`, `/api/certificates`
CRUD standar: `GET /` (publik), `GET /{id}` (publik), `POST /`, `PUT /{id}`, `DELETE /{id}` (JWT).

### Stats - `/api/stats`
`GET /` dan `GET /{key}` publik; `POST /`, `PUT /{key}`, `DELETE /{key}` butuh JWT. Nilai `total_projects` dan statistik berbasis `start_date` dihitung otomatis.

### Upload - `/api/upload`
| Method | Path | Keterangan |
|---|---|---|
| POST | `/api/upload` | Upload image (≤ 5 MB, jpeg/jpg/png/gif/webp) ke R2 |
| POST | `/api/upload/pdf` | Upload PDF (≤ 10 MB) ke R2 |
| DELETE | `/api/upload/{filename}` | Hapus file dari R2 |

Semua butuh JWT. Body multipart.

### Chat AI - `/api/chat`
| Method | Path | Keterangan |
|---|---|---|
| POST | `/api/chat` | Kirim pesan ke CHANGLI-AI (Cerebras + RAG context). Rate limit 50/jam per IP |

### Contact - `/api/contact`
| Method | Path | Keterangan |
|---|---|---|
| POST | `/api/contact` | Kirim pesan → Discord webhook |

### GitHub - `/api/github`
| Method | Path | Keterangan |
|---|---|---|
| GET | `/api/github/activity` | Data aktivitas GitHub (events, user, repos), cache 15 menit |
| GET | `/api/github/heatmap` | Gambar contribution heatmap, `Cache-Control: max-age=3600` |

### XP - `/api/xp`
| Method | Path | Keterangan |
|---|---|---|
| GET | `/api/xp` | Total XP (base + bonus) |
| POST | `/api/xp/tick` | Tambah bonus XP (1–4), cooldown 2 detik per IP |

### Health - `/api/health`
`GET /`, `GET /detailed`, `GET /ping`, `GET /database` - semuanya publik.

## Testing

```bash
cargo test
```

Mencakup unit test untuk logika inti: kalkulasi XP (diverifikasi terhadap referensi Node), months-diff, sanitasi nama file, parser durasi JWT, rate limiter, dan validasi email.

## Deployment

```bash
cargo build --release
```

Hasilnya satu binary statis (`target/release/ditdev_be_rust.exe`). Cukup jalankan binary + mount `.env`, tanpa perlu runtime lain. Disarankan di belakang reverse proxy (nginx/Caddy/Cloudflare) untuk HTTPS.

## Struktur Project

```
ditdev_be_rust/
├── migrations/          # sqlx migrations (001_init, 002_fix_schema)
├── src/
│   ├── main.rs          # bootstrap: config → db → router → serve
│   ├── config.rs        # konfigurasi dari environment
│   ├── error.rs         # error handling → JSON response
│   ├── state.rs         # shared state (Arc<AppState>)
│   ├── util.rs          # helper (parse date, PgTimestamp, months-diff)
│   ├── controllers/     # handler per domain
│   ├── middleware/      # auth (JWT) + rate limiting
│   ├── routes/          # definisi route per domain
│   └── services/        # db, r2, rag, cerebras, discord, github, xp
├── .env.example
└── Cargo.toml
```
