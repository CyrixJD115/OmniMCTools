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
- `src/table.rs` `get_table_entries` returns a tri-state `TableLookup`
  (`Absent` / `Deleted` / `Value`) and `src/db.rs` `get_many_owned` treats a
  tombstone as resolved. Upstream reported a deletion as "not found" and kept
  probing older tables, resurrecting deleted records.
- `src/db.rs` `get_many_owned` selects each table's candidate keys by binary
  search over the sorted request (O(T·log K) instead of O(T·K)). Map tiles
  issue ~10–30k exact keys per batch against ~1,000+ table worlds, where the
  linear filter dominated.
