# OmniMCTools (OMT)

A free, open-source Minecraft world tool suite for **Java & Bedrock Edition** (current stable versions, Windows + Android world sources): world library, **map viewer (2D styles, textured, 3D terrain + 3D blocks)**, **world editor (Amulet-style)**, NBT editor, Java⇄Bedrock converter, world pruner, LevelDB browser, and a Lua extension system.

Native Rust. Clean-room implementation inspired by [Universal Minecraft Tool](https://universalminecrafttool.com) (UMT) — **not affiliated with UMT and not a fork**; no UMT code or data is used.

## Status

Early development. Complete so far (see [PLAN.md](PLAN.md) for the full roadmap and progress ledger):

- **NBT** — big/little-endian codec, SNBT parser + serializer, undo/redo edit engine, format-preserving safe saves with automatic backups
- **World I/O** — Bedrock (LevelDB, byte-exact subchunk/data3D codecs) and Java (Anvil regions, level.dat, playerdata)
- **Converter** — Java⇄Bedrock both directions for chunks (blocks, 3D biomes, heightmaps, block entities) and item stacks, with explicit loss reporting; Bedrock→Java also writes Java entity regions (`entities/r.x.z.mca`). Block-*state* mappings stay registry-blocked for now: names pass through, unmapped properties are dropped and reported, never guessed.
- **Apps** — `omt` CLI (`inspect`/`nbt`/`chunks`/`chunk`/`ldb`/`convert`/`prune`/`render`) and an Iced GUI
- **GUI** — Minecraft-styled modern shell (side rail + top bar, 5 themes, all art generated in code by `omt-art`), world library with search syntax + facet filters + favorites + custom covers, launcher-aware world detection (Java/Prism/MultiMC/Modrinth, mcpelauncher incl. per-profile data dirs, BedrockOnLinux Proton prefixes, Windows GDK/UWP)
- **Map Viewer** — read-only, in place: Classic/Shaded (movable sun)/Topographic/Heightmap/Night/Biomes styles applied instantly, underground (caves) mode, slicer, grids, slime chunks, spawn/players/entities/bookmarks, PNG export, block/chunk/region **rulers**, a **distance measure** (left-drag), a **textured** close-up view and a **3D view** (GPU terrain up to 4000 blocks via wgpu, or real textured blocks: 4× MSAA, mipmapped textures, native-resolution rendering with a Render quality setting and an adjustable Block distance); game-style 3D controls: Normal or Spectator movement (fly through blocks), Drag or Mouse-look controls (click to capture, Esc releases), W A S D / Space / Shift / Ctrl. Compact uNmINeD-style chrome: a container nav (2D/3D, dimension, map style, go-to with spawn/player jumps, a Tools menu for the other tools, PNG export), a Photoshop-style tool rail with quick toggles, and a collapsible tabbed side panel (View · Style · Layers · Find · Export). Keyboard: arrows pan (Shift 1 block, Ctrl 256), `+`/`−` zoom, `*` resets, Ctrl+G cycles the chunk grid.
- **World Editor (alpha / experimental)** — select a box on the map: fill, replace, clear, analyze, copy/paste with rotate/flip, biome painting, entity listing/removal, click-to-select in 3D, export/import `.mcstructure` and Java structure `.nbt`, 3D preview of the selection, with undo — on a safe workspace copy, both editions
- **Real textures** — read from the Minecraft you already have installed (`.brarchive` packs of BedrockOnLinux / mcpelauncher / Windows); OMT never ships Mojang's assets (an optional in-app download fetches Mojang's official `bedrock-samples` into your cache)

## Build

Requires Rust 1.88+.

```sh
cargo build --workspace
cargo test --workspace
```

## GUI

```sh
cargo run --release -p omt-gui                      # Home
cargo run --release -p omt-gui -- --page worlds     # world library
cargo run --release -p omt-gui -- --app map <world> # Map Viewer
cargo run --release -p omt-gui -- --app editor <world> # World Editor
```

## CLI examples

```sh
# Inspect a world (either edition)
omt inspect <world-folder>

# Decode everything in a world and report problems (run after editing)
omt verify <world-folder>

# Convert chunks Java → Bedrock (target must be a disposable copy!)
omt convert chunks --java <java-world> --bedrock <bedrock-copy> [--dim nether] [--dry-run]

# Convert chunks Bedrock → Java (--java is the target, created when missing)
omt convert chunks --to-java --bedrock <bedrock-world> --java <new-or-existing-folder> [--dry-run]

# Convert one item stack (SNBT on stdin/argument; losses go to stderr)
omt convert item-stack --to-bedrock '{id: "minecraft:diamond_sword", Count: 1b, tag: {…}}'
omt convert item-stack --to-java '{Name: "minecraft:apple", Count: 3b}'
```

## Acknowledgements

Development of OmniMCTools is assisted by [Claude](https://claude.com/claude-code) (Anthropic), used as an AI coding assistant.

Vendored third-party crates (each with its license and an `OMT_PATCH.md` describing the change): `bedrock-leveldb` (MIT / Apache-2.0), `iced_tiny_skia` and `iced_winit` (MIT, © Héctor Ramón, Iced contributors).

## License

Licensed under GPL-3.0-or-later — see [LICENSE](LICENSE).
