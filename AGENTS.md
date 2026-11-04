# AGENTS.md — guidance for agents working on OmniMCTools (OMT)

Read this before changing anything. The authoritative spec and progress
ledger live in `PLAN.md` at the repository root (next to this file).
This file covers the rules that are easy to violate when moving fast.

## What this project is

Free, open-source, **clean-room** Rust reimplementation of Universal
Minecraft Tool (UMT) for **latest-stable** Java & Bedrock Editions (Windows +
Android sources). Library + CLI (`omt`) + Iced GUI. Repository:
<https://github.com/CyrixJD115/OmniMCTools> (GPL-3.0-or-later).

## Hard rules (never violate)

1. **Clean-room.** UMT is a behavioral reference only. Formats, observable
   behavior, and REA findings may be documented and cited; decompiled UMT
   code is never transcribed, ported, or paraphrased into OMT source.
   Reverse-engineering evidence (decompiles, dumps) lives in `../RE/` and
   must **never** be committed or shipped with the product.
2. **No placeholders.** Every function does the real thing or returns a
   typed error. `todo!()`, fake constants pretending to be data, and
   "would-be implemented with registry" stubs that *silently* drop data are
   forbidden. Data that doesn't exist yet (e.g. block-state mappings before
   `client.jar` is pinned) is handled through the loss ledger
   (`omt-convert::loss`) so nothing is dropped silently.
3. **Never claim unverified acceptance.** A milestone is DONE only after the
   PLAN.md acceptance criteria are verified (tests run, fixtures round-trip,
   output opens). Mark partial work as partial in the PLAN.md ledger.
4. **Fixtures and personal data never enter git.** `world_ref/`, test
   worlds, player data: ignored (see `.gitignore`); fixture-gated tests
   `#[ignore]` and read paths from env (`OMT_BEDROCK_FIXTURE`) or fall back
   to `../../world_ref/`.
5. **Iced for the GUI.** `iced` 0.14 mandatory; egui is forbidden by project
   decision. API pitfalls are recorded in the agent memory note
   (`iced-0.14-api-notes`), not relearned per session.
6. **No unsafe.** Workspace forbids `unsafe_code`; edition 2024; keep
   `cargo build/test --workspace` at zero warnings.

## File size budget

- **Target: every source file stays under 700 lines.** Hard ceiling: 900.
- Prefer splitting a file over growing past the target: extract a module
  (types / parse / serialize), or move `#[cfg(test)]` bodies into
  `tests/`. Doc comments count toward the budget; generated files and
  fixtures don't.
- An exception needs a one-line justification in the PLAN.md ledger.
- Known current exceptions (split opportunistically when touching them):
  `crates/omt-convert/src/items.rs` (~1250), `crates/omt-nbt/src/edit.rs`
  (~830).

## Layout

```
crates/omt-nbt        NBT codec, SNBT, edit engine (M0/M2)
crates/omt-world      edition-neutral domain: Chunk/Slice/BlockState, stores
crates/omt-versions   version registry (RON pins)
crates/omt-java       Anvil regions, Java world store (M1)
crates/omt-bedrock    LevelDB keys, subchunk/data3D codecs (M1)
crates/omt-convert    Java <-> Bedrock conversion + loss ledger (M3/M4)
crates/omt-cli        the `omt` binary
crates/omt-gui        Iced app (M2 editor; grows per milestone)
tools/registry-gen    (M3) registry generation from pinned client.jar
```

`omt-world` is the shared floor — no crate may depend on a sibling above it
except `omt-convert` (which composes java/bedrock codecs) and the binaries.

## Working agreements

- **Read PLAN.md §0 first.** It records as-built state, verified format
  facts (which **override** older public docs), and open risks. When you
  establish a new format fact empirically (probe against `world_ref/`),
  add it there with the probe that proved it — `crates/omt-bedrock/examples/`
  keeps the probes.
- **Testing bar:** unit tests next to code + byte-faithful round trips for
  codecs + at least one end-to-end path per milestone. Java fixture still
  missing: Java-side facts marked "pending Java fixture" in PLAN.md are
  assumptions to verify, not facts — say so in doc comments.
- **Errors are values.** Malformed input is a typed error, never a panic;
  `expect()` only for genuinely impossible states.
- **Commits:** small and conventional (`feat:`, `fix:`, `docs:`, `chore:`).
  Never commit `target/`, fixtures, or RE evidence.
- **Host quirks:** `python3` is a shell shim here — use `/usr/bin/python3`;
  filter unrelated shell noise with `grep -vE "zcode|electron|appimage"`;
  shell cwd resets between commands — use absolute paths.

## Milestone discipline

Work milestone-by-milestone per PLAN.md §4/§9. When a milestone's criteria
are met: run the full workspace build+tests, stamp the ledger entry with the
date and "As built" notes (including deviations), and update §7/§8 statuses.
M3 is current: chunk pipeline done (see ledger); `tools/registry-gen` and
the semantic block-state mapping table are blocked on a **pinned
`client.jar`** and a **real Java fixture world** — drop both into
`world_ref/` to unblock.
