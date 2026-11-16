# OMT patch to bedrock-leveldb 0.3.1

Upstream: https://github.com/BE-Community-Dev/bedrock-leveldb (MIT OR Apache-2.0,
license files kept alongside). Vendored unmodified except:

- `src/db.rs` `flush_locked`: the rewritten database view is split into
  ~2 MiB sorted, non-overlapping native tables instead of a single table.
  Reason: with one table per database, point reads cost ~7 ms on a 36 MB
  world (vs ~0.33 ms on Mojang-written ~2 MB tables) — measured with
  `crates/omt-bedrock/examples/getbench.rs`. Reads were already range-
  filtered per table, so multiple tables work unchanged.
- `Cargo.toml`: the `[[bench]]` target was dropped (benches not vendored).
