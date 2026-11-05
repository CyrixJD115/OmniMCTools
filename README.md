# OmniMCTools (OMT)

A free, open-source Minecraft world tool suite for **Java & Bedrock Edition** (current stable versions, Windows + Android world sources): world browser, NBT editor, Java⇄Bedrock converter, world pruner, LevelDB browser, and a Lua extension system.

Native Rust. Clean-room implementation inspired by [Universal Minecraft Tool](https://universalminecrafttool.com) (UMT) — **not affiliated with UMT and not a fork**; no UMT code or data is used.

## Status

Early development. Complete so far (see [PLAN.md](PLAN.md) for the full roadmap and progress ledger):

- **NBT** — big/little-endian codec, SNBT parser + serializer, undo/redo edit engine, format-preserving safe saves with automatic backups
- **World I/O** — Bedrock (LevelDB, byte-exact subchunk/data3D codecs) and Java (Anvil regions, level.dat, playerdata)
- **Converter** — Java⇄Bedrock both directions for chunks (blocks, 3D biomes, heightmaps, block entities) and item stacks, with explicit loss reporting; Bedrock→Java also writes Java entity regions (`entities/r.x.z.mca`). Block-*state* mappings stay registry-blocked for now: names pass through, unmapped properties are dropped and reported, never guessed.
- **Apps** — `omt` CLI (`inspect`/`nbt`/`chunks`/`chunk`/`ldb`/`convert`) and an Iced GUI with a tabbed NBT tree editor

## Build

Requires Rust 1.88+.

```sh
cargo build --workspace
cargo test --workspace
```

## CLI examples

```sh
# Inspect a world (either edition)
omt inspect <world-folder>

# Convert chunks Java → Bedrock (target must be a disposable copy!)
omt convert chunks --java <java-world> --bedrock <bedrock-copy> [--dim nether] [--dry-run]

# Convert chunks Bedrock → Java (--java is the target, created when missing)
omt convert chunks --to-java --bedrock <bedrock-world> --java <new-or-existing-folder> [--dry-run]

# Convert one item stack (SNBT on stdin/argument; losses go to stderr)
omt convert item-stack --to-bedrock '{id: "minecraft:diamond_sword", Count: 1b, tag: {…}}'
omt convert item-stack --to-java '{Name: "minecraft:apple", Count: 3b}'
```

## License

Licensed under GPL-3.0-or-later — see [LICENSE](LICENSE).
