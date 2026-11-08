# OmniMCTools fuzz targets

`cargo-fuzz` targets for the parsing surfaces that touch untrusted world data.
This directory is its **own cargo workspace** (cargo-fuzz layout) and is
deliberately not a member of the repo workspace — fuzzing needs the **nightly**
toolchain.

## Setup (once)

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz
```

## Targets

| Target | Surface | Invariant |
|---|---|---|
| `fuzz_nbt_reader` | `omt_nbt::read` (BE/LE) + `Container::decode` sniffing | typed error or clean parse — never panic (MAX_DEPTH 512 enforced) |
| `fuzz_region_reader` | `omt_java::RegionFile::from_bytes` + `read_chunk` | garbage offsets / sector counts / truncation / bad compression → typed errors |
| `fuzz_subchunk_codec` | `omt_bedrock::Subchunk` (v8/v9) + `Data3d` decode→encode | decode⇄encode round-trips or errors; never hangs (incl. bits==0 single-entry) |
| `fuzz_leveldb_key` | `omt_bedrock::keys::{decode_key, chunk_record, subchunk_key, digp_key, actor_key}` | any bytes classify without panicking; encoder→decoder round-trips |

Seed corpora live in `seeds/<target>/` — one entry per interesting byte shape
(valid BE/LE NBT, gzip/zlib/raw containers, truncated/malformed variants,
region headers (empty / one-chunk / garbage / truncated), subchunk v8/v9 +
data3D shapes incl. bits==0, LevelDB key shapes for every `DbKey` variant).
These are tracked in git; the *grown* corpus (`corpus/`) and `artifacts/` are
gitignored — always pass a writable corpus dir so runs never rewrite seeds.

## Run

```sh
cd fuzz
mkdir -p corpus/{fuzz_nbt_reader,fuzz_region_reader,fuzz_subchunk_codec,fuzz_leveldb_key}
cargo fuzz run fuzz_nbt_reader     seeds/fuzz_nbt_reader corpus/fuzz_nbt_reader     -- -max_total_time=60
cargo fuzz run fuzz_region_reader  seeds/fuzz_region_reader corpus/fuzz_region_reader  -- -max_total_time=60
cargo fuzz run fuzz_subchunk_codec seeds/fuzz_subchunk_codec corpus/fuzz_subchunk_codec -- -max_total_time=60
cargo fuzz run fuzz_leveldb_key    seeds/fuzz_leveldb_key corpus/fuzz_leveldb_key    -- -max_total_time=60
```

Smoke runs (~60 s per target) are the expected cadence locally; longer
campaigns are opt-in. Crashers land in `artifacts/` — triage, then fix or
file under PLAN.md §7. Any input that makes a target panic is a real bug:
the crates' contract is *typed errors, never panics*. (2026-11-08 smoke
runs found one — an empty-palette packed storage — fixed in omt-bedrock
with regression tests; see PLAN.md §4.)

The CI pipeline only *builds* these targets on nightly (it never runs them).
