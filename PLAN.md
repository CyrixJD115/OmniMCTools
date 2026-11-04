# PLAN.md — OmniMCTools

**Working name:** `OmniMCTools`, short form **OMT**, binary **`omt`**. Repo: `github.com/CyrixJD115/OmniMCTools`.
*Naming note:* OMT deliberately mirrors UMT (Universal → Omni, same one-letter-initialism shape) — the positioning is "the free, open alternative to UMT." The name is clear: no existing GitHub project, and the only OMT in Minecraft space is OpenModularTurrets (a Forge mod, different niche). crates.io's bare `omt` is an unrelated packer — irrelevant locally; published crates are `omt-*`. **Framing rule:** the README must state this is an independent clean-room implementation, inspired by UMT — not affiliated, not a fork.

**Mission:** a free, open-source, native Rust Minecraft **world tool suite** for the **current stable versions** of Java Edition and Bedrock Edition (Windows & Android world sources): world browser, NBT editor, Java⇄Bedrock converter, world pruner, LevelDB browser, and a Lua extension system — functionally modeled on Universal Minecraft Tool (UMT), clean-room implemented from RE research.

**Grounding documents** (in [`../RE/`](../RE/)):
- [`UMT_Research_Report.md`](../RE/UMT_Research_Report.md) — RE evidence & architecture
- [`UMT_Rust_Port_Guide.md`](../RE/UMT_Rust_Port_Guide.md) — UI/theme spec, format cookbook, crate mapping
- [`UMT_Port_Guide_Addendum.md`](../RE/UMT_Port_Guide_Addendum.md) — chunk structs, edition dispatch, console codec, corruption repair, virtual paths
- [`umt_re_evidence/`](../RE/umt_re_evidence/) — decompiled functions, string corpora, REA snapshot pointer *(local-only — clean-room rule: RE evidence is never committed or distributed with the product)*

---

## 0. Progress ledger (updated 2026-10-23 — **M0–M2 done, M3 is next**)

| Milestone | Status | Evidence |
|---|---|---|
| M0 NBT+world foundations | ✅ done | `cargo test --workspace` green; workspace at `./` with crates omt-nbt/-world/-versions/-cli/-gui; `registry.ron` pinned to fixture truth (Bedrock **1.26.52**, storage ver 10) |
| M1 world open/browse | ✅ done | Bedrock: 17,511 subchunks / 71.7M blocks / 0 errors, byte-identical re-encode (`OMT_BEDROCK_FIXTURE=<world> cargo test -p omt-bedrock --test fixture`). Java: Anvil rw + synthetic world round-trip (`cargo test -p omt-java`). CLI: `omt inspect/nbt/chunks/chunk/ldb` verified against `world_ref/` |
| M2 NBT editor | ✅ done | `omt-nbt`: `from_snbt` (vanilla-syntax parser w/ position errors), `edit.rs` (`TagPath`/`EditOp`/`UndoStack` w/ coalescing), `document.rs` (format-preserving load/save, atomic temp+rename, `.omt_backup`). `omt-gui`: tabbed Iced editor (tree browse/expand, inline SNBT edit, add/del/rename/move, Ctrl+Z/Y/S/F hotkeys, search+next, SNBT copy/paste via clipboard, dirty ●, green/amber change highlight, rfd file picker). CLI: `omt nbt --set PATH --value SNBT [--out]`. 58 workspace tests; Bedrock fixture level.dat edit→save→reload verified end-to-end (backup byte-identical). Not verified: in-Minecraft load (no runtime), real Java fixture still absent |
| M3–M8 | not started | **Blocked for M3: no real Java fixture world yet** — drop one into `world_ref/` or `tools/world-fixtures/`; also pin a `client.jar` for registry generation |

Open items carried forward: parsedat-bedrock preamble-robustness port into `BedrockLevelDat::parse`; unknown 1.26 chunk tags 0x35/0x3f/0x40/0x41 kept raw (REA backlog); rename/undo sequences make change-highlight paths best-effort; full copy-on-open *workspace* model (§3.3) lands with M3 write paths (M2 saves are per-file and already safe).

**2026-11-03 REA pass (pre-M3/M4):** converter-behavior unknowns resolved against the binary — findings + evidence in [`../RE/UMT_Converter_RE_Findings.md`](../RE/UMT_Converter_RE_Findings.md). Highlights: `skipProtoChunks` semantics captured (default ON; era-aware predicate; drop-at-parse incl. entities); entity handling = universal lowercase `entities` ⇄ Java entity-region root `{DataVersion, Position:[I;x,z] (2 ints), Entities}`; border-chunk deferral is UMT-pipeline artifact (OMT skips it); **subchunk XZY order verified** (risk retired); item `components`/`tag` conversion is **fully DB-driven** in UMT (`database.ndb` + `_tagModifiers` rule column — no key literals in the binary), so M4 designs our own rule schema (see updated §4 M4 + §7).

**2026-11-04 codified into Rust (`omt-convert` crate, M3/M4 groundwork — fixture-independent):** `java_stack_to_bedrock`/`bedrock_stack_to_java` over `omt_nbt::Tag` (both Java eras: `tag` ≤1.20.4 and `components` ≥1.20.5; custom names/lore, enchantments via pluggable `EnchantIdMap`, damage coercions, written/writable books, `Slot`), every lossy mapping returned in a `ConversionReport` ledger (rows per findings Task A: unsupported components dropped+reported, `WasPickedUp` reported when informative, mixed-era policy `PreferComponents`/`PreferTag`/`Reject` — UMT `mixedComp` precedent); `is_proto_chunk` + `ProtoFilter` (Task B predicate verbatim, default ON, entity/BE counts surfaced); `entities_key`/`find_entities`/`entity_region_root` (Task C shapes; border special-case deliberately skipped per findings conclusion); `subchunk.rs` XZY docs now cite the verified addresses instead of the assumption. CLI: `omt convert item-stack --to-bedrock|--to-java [--legacy-java]` (SNBT arg/stdin → SNBT stdout, losses to stderr). Bedrock stack shape grounded in the 1.26.52 fixture (`Name/Count/Damage/WasPickedUp`, vault `key_item`); no display/ench/book items exist in fixtures — those writer shapes are marked OMT-design in code docs. 85 workspace tests green (58 before), 0 warnings. Still blocked on Java fixture + pinned `client.jar` for registry generation and the chunk pipeline.

**2026-11-04 (2) M3 converter core landed — the per-chunk pipeline is real and fixture-verified end-to-end:** `omt-convert` now carries `rules` (OMT-owned RON rule table: block rules empty-by-design until registry-gen, seeded stable biome-id table, probe-derived `block_palette_version`), `chunk_java` (modern-era Java chunk sidecar: sections, heightmaps, block entities; legacy `Level` layout is a typed error), `chunk_bedrock` (sidecar → raw LevelDB records: v9 subchunks, data3D with mapped biomes, version 42 + finalized 2, LE block entities; all-air sections omitted; delete-stale handled by the apply step), and `pipeline` (rayon `read → parse → remap → serialize` + `apply_conversions` into a writable target; per-chunk failures never abort a world run). CLI: `omt convert chunks --java <dir> --bedrock <dir> [--dim] [--rules] [--keep-proto] [--dry-run]`. **Two latent Java-side bugs fixed** (both unverifiable until now): `unpack_java_indices` used the pre-1.16 continuous-stream layout (moved to `omt_world::packing`, rewritten non-spanning per wiki.vg), and `omt-bedrock` copied XZY indices into the universal model verbatim so `Chunk::block_at` was transposed for Bedrock reads (normalized XZY→YZX at the store boundary; `Slice` doc corrected). **Fixture e2e test** (`java_to_bedrock_fixture.rs`, `#[ignore]`, fixture-gated): synthetic Java world → temp copy of the 1.26.52 fixture → reopened: blocks correct through both permutations, recomputed heightmap, mapped biomes, LE chest NBT, **untouched fixture columns byte-identical**, proto chunk skipped. 103 workspace tests green, build 0 warnings. Still blocked for the *registry* half: Java fixture + pinned `client.jar`. `AGENTS.md` added (clean-room rules, <700-line file budget, workflow).

---

## 1. Scope — what we support and what we don't

### 1.1 Supported (the whole product)

| Area | IN scope |
|---|---|
| **Java Edition** | **Latest stable line only** (currently the 1.21.x/26.x era — see §2 "version registry"). Read + write. |
| **Bedrock Edition** | **Latest stable line only**, world sources: **Windows 10/11** (`%LOCALAPPDATA%\Packages\Microsoft.MinecraftUWP_8wekyb3d8bbwe\LocalState\games\com.mojang\minecraftWorlds\`) and **Android** (`games/com.mojang/minecraftWorlds/` pulled via user copy or `adb`, plus generic folders) and **`.mcworld` / `.mctemplate`** archives. |
| **World containers** | Java world folders + zips; Bedrock folders + `.mcworld`/`.mctemplate`. |
| **Dimensions** | Overworld, Nether, The End — all three, both editions. |
| **Modules** | World browser · NBT editor (undo/redo/search/hex) · Converter (Java⇄Bedrock, latest⇄latest) · Pruner (Box/Circle/Polygon/Split/Time/Optimize/Void) · Chunk Locator 2D map + Slicer · LevelDB browser · Lua extensions. |
| **Platforms for the tool itself** | Desktop: Windows + Linux (64-bit). CLI-first core, GUI on top. |

### 1.2 Explicitly OUT of scope (say no early, stay fast)

| OUT | Rationale / behavior instead |
|---|---|
| Legacy consoles (PS3/X360/Wii U) | UMT's obfuscated secret sauce, tiny audience → not worth it. Never. |
| Old-version worlds & up/downgrades | If input world is older than our minimum (§2), show "open & re-save it in the current game once, then retry." Minecraft itself is the upgrader. No format-version ladders. |
| Accounts, plans, DRM, telemetry, updater-server | Dropped entirely (UMT's `doDRM`/`/updateActivityLog` layer documented in addendum §7 and deleted). |
| Modded worlds | Vanilla only; detect mod blocks (`minecraft:`-unknown ids / huge registries) → warn, best-effort, no mapping. |
| Realms/Education/`*.mctemplate` authoring beyond open | Reading templates = yes (it's a zip). Writing them = out. |
| Player data conversion across editions | Account-bound, impossible in principle (UMT skips it too). Player NBT *editing* in-world = yes. |
| macOS/iOS Bedrock sources | Later, if ever (folder layout is the same — low cost but not now). |
| 3D world editor ("Commander"/"3D Editor") | Doesn't even exist in UMT's build (stubs). Out. |

### 1.3 "Latest" policy in one line
One Java format generation (**1.18+ sections/`block_states`**, in practice 1.20.5+), one Bedrock generation (**1.18+ subchunks with 3D biomes**, in practice 1.21+), and a tiny **version registry file** we bump each release instead of compiling in ten years of history.

---

## 2. Version registry (the trick that keeps scope small)

`crates/omt-versions/registry.ron` — single source of truth, bump per Minecraft release. **Exists since M0; current pins are fixture-derived** (Bedrock `lastOpenedWithVersion` 1.26.52.3.0, storage version 10, chunk version 42). Java pins are provisional until M3 pins a `client.jar` (the enforced read floor 2865 = 1.18 is what gates reads today).

```rust
pub struct EditionVersions {
    pub java_data_version: u32,        // e.g. current stable DataVersion
    pub java_release: String,          // "26.2"
    pub java_min_data_version: u32,    // floor we READ (1.18 = 2865); below → friendly error
    pub bedrock_level_version: [u32;5],// level.dat storageVersion + minCompatibleClientVersion
    pub bedrock_subchunk_palette: u8,  // current palette version (v8/v9)
    pub bedrock_release: String,       // "1.26.52" (fixture-pinned)
    pub bedrock_min_chunk_version: u8, // floor we READ (1.18 subchunk split)
}
```
Rules: writer always stamps the **current** entry; reader accepts `>= min`; anything below min → per-world warning + refuse to *write* (read/browse/NBT-edit still allowed — it's harmless). This replaces UMT's 48-byte format-descriptor ladders with two constants + a floor check (see addendum §2 for what we're simplifying away).

---

## 3. Architecture

### 3.1 Workspace (as built through M2; ✅ = exists)

```
Obsidian Tool/                 # Cargo workspace root (PLAN.md lives here)
├── crates/
│   ├── omt-nbt/        ✅ # BE+LE NBT, Tag enum, gzip/zlib/raw, SNBT (parse+render),
│   │                      #   tree/hexdump, edit engine (TagPath/EditOp/UndoStack),
│   │                      #   NbtDocument (format-preserving safe save + .omt_backup)
│   ├── omt-world/      ✅ # domain: Chunk/Slice/BlockState/BedrockBiomeSlot, WorldStore trait,
│   │                      #   VPath namespace, edition detect + WorldSummary
│   ├── omt-versions/   ✅ # registry.ron + floor checks (fixture-pinned)
│   ├── omt-java/       ✅ # Anvil .mca reader/writer (lenient), level.dat (Data nesting),
│   │                      #   playerdata, DIM paths, sections→Chunk decode
│   ├── omt-bedrock/    ✅ # LevelDB store (bedrock-leveldb), key codec, subchunk v8/9 codec
│   │                      #   (byte-exact), data3D per-slot biomes, digp/actorprefix
│   ├── omt-registry/     # block/item/biome/entity id database (generated) — M3
│   ├── omt-convert/     ✅ # M3/M4 groundwork: item stacks (tag+components eras ⇄ Bedrock, loss
│   │                      #   ledger), skipProtoChunks filter, entity-region assembly; rest M3
│   ├── omt-prune/        # layer model + rasterizer + InhabitedTime filter — M5
│   ├── omt-render/       # 2D chunk tiles, biome tint — M6
│   ├── omt-script/       # mlua extension host (UMT-compatible API) — M7
│   ├── omt-cli/        ✅ # `omt`: inspect / nbt (dump+set) / chunks / chunk / ldb / convert
│   └── omt-gui/        ✅ # Iced 0.14: Home + tabbed NBT editor (M2); grows per milestone
└── tools/
    ├── registry-gen/       # builds omt-registry data from pinned client.jar + Bedrock data — M3
    └── world-fixtures/     # test-world harness (see §8) — created when fixtures arrive
```
Key dependency decisions (as built): `bedrock-leveldb 0.3.1` (raw-zlib Bedrock LevelDB — retires §7's leveldb risk), `iced 0.14`, `rfd 0.17` (native file dialogs), `clap 4`, `flate2`.

### 3.2 Core types (as built in `omt-world`; originally from RE, latest-only simplified)

```rust
pub enum Edition { Java, Bedrock }
pub enum Dimension { Overworld, Nether, End } // id(): 0/1/2 (Bedrock dim byte)

// UMT's virtual-path idea (addendum §5) — one API over both stores
pub enum VPath { DbChunk { dim, x, z, tag: Option<u8> }, DbRaw(Vec<u8>), File(String) }
pub enum WorldNode {
    Chunk { dim: u8, x: i32, z: i32 },
    LevelDat, PlayerFile(String), DbKey(Vec<u8>), EntityRegion { dim, x, z },
}
pub trait WorldStore: Send + Sync {
    fn read_chunk(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<Chunk>>;
    fn write_chunk(&self, dim: u8, x: i32, z: i32, chunk: &Chunk) -> WorldResult<()>;
    fn delete_chunk(&self, dim: u8, x: i32, z: i32) -> WorldResult<()>;   // pruner
    fn iter_chunks(&self, dim: u8) -> WorldResult<Box<dyn Iterator<Item=(i32,i32)> + '_>>;
    fn read_raw(&self, node: &WorldNode) -> WorldResult<Vec<u8>>;
    fn flush(&mut self) -> WorldResult<()>;
}

// Chunk modeled on UMT's decoded structs (addendum §1), latest-only:
pub struct Chunk {
    pub x: i32, pub z: i32, pub dim: u8,
    pub data_version: u32,                    // Java DataVersion; Bedrock chunk_version (42)
    pub status: Option<String>,               // Java "Status"; Bedrock synthesizes "full"
    pub inhabited_time: i64,                  // pruner Time layer
    pub slices: BTreeMap<i8, Slice>,          // Y-index (−4..=19)
    pub heightmaps: Vec<(String, Vec<u64>)>,  // Java packed; Bedrock 2D u16 ("bedrock_2d")
    pub biomes_3d: bool,
    pub bedrock_biomes: Option<Vec<BedrockBiomeSlot>>, // data3D per-slot (below)
    pub block_entities: Vec<Tag>, pub entities: Vec<Tag>,
    pub block_ticks: Vec<Tag>, pub fluid_ticks: Vec<Tag>,
}
pub struct Slice { pub palette: Vec<BlockState>, pub indices: Vec<u16> } // 4096, XZY (Bedrock)

// Bedrock blocks normalize to name+states through each world's OWN palette
// (runtime ids are never treated as globally stable — §7).
pub struct BlockState { pub name: String, pub properties: Vec<(String, String)> }

// data3D 0x2B per-slot biomes (slot i covers Y i*16−64; 0xFF = copy from below)
pub enum BedrockBiomeSlot { NonExistent, Storage { palette: Vec<i32>, indices: Option<Vec<u16>>, bits: u8 } }
```

### 3.3 Safety model (copy UMT's proven workflow)
Open = **copy world to workspace temp dir** (`.omt-workspace/`), all edits hit the copy, Save = write back to origin + keep `_omt_backup` of the original. Bedrock always copies (LevelDB compaction danger — UMT's warning strings verbatim in report). Crash = user loses nothing.

**As implemented so far (M2, per-file saves):** `NbtDocument::save` validates the document serializes *first*, copies the pristine original to `<name>.omt_backup` **once**, writes `.name.omt_tmp`, then renames over the target — an interrupted save can never leave a half-written original, and a save failure leaves the original untouched with a reported error. The full world-folder copy-on-open workflow lands with the M3+ write paths (chunk writing, converter, pruner).

---

## 4. Feature plan by module (features · functions · RE grounding · REA follow-ups)

Format per module: **[F]** features, **[FN]** internal functions to build, **[RE]** what we already know from RE, **[REA]** what to decompile with REA when implementing (addresses from our Ghidra project; snapshot at `/tmp/umt_re/rea_snap.zip`; after a ZCode restart the REA MCP tools attach directly — `open_binary`/`analyze_function`).

### M0 — `omt-nbt` + `omt-world` foundations — ✅ DONE
- **[F]** Parse/serialize NBT both endiannesses; gzip/zlib/raw containers; Bedrock level.dat 8-byte header; hex dump helper.
- **[FN]** `read_nbt`/`write_nbt` (const-generic byte order), `Tag` enum, container sniffing (magic + compression byte), fuzz target.
- **[RE]** UMT Tag tree = ids 0–12 exactly (report §4.4); `isLE` split confirmed.
- **[REA]** optional: NBT varint-free confirmation for current Bedrock — `0x4C0F60` neighbors.
- **As built:** 21 tests incl. seeded round-trips + mutation fuzz-smoke (`--ignored` for the long run); public streaming `Reader` reused by omt-bedrock for embedded NBT (palettes, actors). *Found & fixed: NBT `ByteArray` is signed (`Vec<i8>`, not `Vec<u8>`).*

### M1 — World open/browse (Java + Bedrock stores) — ✅ DONE
- **[F]** Open Java folder/zip; open Bedrock folder/`.mcworld`/`.mctemplate`; detect edition + version (Java `DataVersion`; Bedrock `level.dat` storageVersion + `MinimumCompatibleClientVersion`); world tree **Files / Players / Maps**; `levelname.txt`/`level.dat` summary; world thumbnail (`world_icon.jpeg` or generated); player-name resolution via Mojang session API (cached, rate-limited).
- **[FN]** Anvil reader/writer (4 KiB sectors, 8 KiB header, compression schemes 1/2/3 + lz4 if current Java uses it — verify against a real current world), **lenient mode** (region dedup / bad-offset skip — UMT corruption strings, addendum §4); LevelDB open (rusty-leveldb + raw-zlib block support), key codec `[x LE][z LE][dim][tag]`, subchunk `[…0x2F][y]`, `data2D/3D`, `~local_player`, `actorprefix`/`digp`, dimension encoding.
- **[RE]** `ChunkTaskLoad::run 0x45A230` (region paths + `:DB/:CHUNK`), UMT's leveldb build (`leveldb.BuiltinBloomFilter2`, ZlibCompressorRaw — raw zlib, not snappy: pick leveldb crate accordingly or patch).
- **[REA]** `0x45CB70`+neighbors for exact region-sector state machine; LevelDB key-build function (search xrefs to `"Overworld"` string) to confirm current key layout incl. dimension encoding.
- **As built / deviations:** LevelDB via **`bedrock-leveldb 0.3.1`** (has raw-zlib — the rusty-leveldb plan was dropped); `.mcworld`/zip opening still TODO (folders only); Mojang name resolution not yet wired. Java `level.dat` nests fields under a root `Data` compound (handled). Anvil writer emits zlib scheme; lz4 unverified until a real Java fixture exists.
- **Verified 1.26.52 format facts** (empirically, byte-exact on fixtures — these override older public docs for M3+):
  - Chunk keys: base `[x LE][z LE]` (+ dim byte 1/2 for nether/end). Tags: **0x2B** data3D · **0x2C** chunk version (byte 42) · **0x2F+y** subchunk · **0x31** block entities (**sequence** of LE NBT roots, one per entity) · **0x33** block ticks · **0x36** finalized state (i32) · **0x3A** fluid ticks · **0x77** structure refs · `digp`+base → actor digests · `actorprefix`+8B id → actor NBT. Unknown raw-kept: 0x35/0x3F/0x40/0x41.
  - Subchunk payload: `[ver=9][storage_count][y byte]` then per storage `[header bits<<1|runtime][word-aligned packed 4096 indices][palette]` — packing is **word-aligned int32** (`floor(32/bits)` values/word), palette = u32 count + full NBT-root entries (`0a 00 00` + `name`/`states`/`version`), and **bits==0 means one palette entry with no count prefix**.
  - data3D: 512B heightmap (256×u16) + per-subchunk slots: `01 <i32 biome>` uniform, or `[bits<<1|1][packed 4096][i32 count][i32 ids]`, or `FF` = same-as-below. 24 slots on these worlds. **Biomes are per-block (4096), not 4×4×4.**
  - data3D **heightmap semantics** (probe `omt-bedrock/examples/heightprobe.rs`, 2026-11-04): value = topmost **motion-blocking/liquid** block − (−64) + 1, column order **z-major** (`index = z*16 + x`, x fastest) — 10200/10240 columns exact on fixture "Base" (remainder: modded `spark:*` non-colliding decor blocks). Java's `MOTION_BLOCKING` heightmap uses the **identical numeric encoding**, so Java→Bedrock heightmaps copy 1:1 (Java-side column order still assumed z-major — pending Java fixture).
  - Block palette entries: every checked entry (6228/6228 across the fixture) carries `version` int **18168865** = packed 1.21.60.33 — OMT writes this as the default for generated entries (`rules::DEFAULT_BLOCK_PALETTE_VERSION`).

### M2 — NBT editor — ✅ DONE
- **[F]** Tree view (tag icons per guide §3.2 set), inline edit per type, add/remove/rename/move tags, **undo/redo** (command pattern, coalescing — addendum §6), search (find/find-next across tree), copy/paste tags as **SNBT-style text**, change highlighting (added=green / edited=amber), per-world multi-tab, standalone `.dat` files, hex fallback view.
- **[FN]** `EditOp` enum + `UndoStack`; SNBT (de)serializer (bonus: round-trips with text).
- **[RE]** UMT `NBTTreeView`/`TagItemWidget` anatomy + QSS in guide §3; `CharCommand` = QUndoCommand interface.
- **[REA]** copy/paste text format: xref `copy`/`paste` handlers near `TagItemWidget` meta-strings if we want byte-compatible clipboard text (low priority — SNBT is fine).
- **As built:** everything above except tag-icon set (text type labels) and in-GUI hex fallback (`omt nbt --hex` covers it). Acceptance: Bedrock fixture `level.dat` gamerule edit + undo verified programmatically & via CLI end-to-end (backup byte-identical); Java side synthetic only — **real Java fixture still absent**; in-game load unverified (no runtime here).

### M3 — Registry + converter core (blocks/biomes/height) — **converter core DONE 2026-11-04; registry-gen still blocked (Java fixture + client.jar pin)**
- **[F]** Java⇄Bedrock chunk conversion at latest: blocks (state⇄runtime-id), 3D biomes, heightmaps (recompute MOTION_BLOCKING family), section split (Java 16-block sections ⇄ Bedrock 16-block subchunks, Y −64..319 aligned — **both editions share the 1.18+ coordinate space, no shifting needed**, biggest simplification vs UMT), `Status` mapping, options: skip-partial chunks, include entities/tile-entities/ticks toggles, dimension toggles. **As built (Java→Bedrock direction):** blocks (1:1 name passthrough; stateless only until registry lands — stateful blocks drop properties *with a loss row*), 3D biomes (seed id table, unmapped → ocean + loss row), heightmaps (Java `MOTION_BLOCKING` copies 1:1 per the probe fact; WORLD_SURFACE fallback; recompute-from-blocks last), section⇄subchunk split with YZX⇄XZY permutation, proto-chunk skipping (UMT default ON), per-chunk loss ledger aggregated per dimension. Ticks and entities are deferred (loss rows; entities = M4 per plan). Bedrock→Java direction: not started.
- **[FN]** `tools/registry-gen`: extract Java states from pinned `client.jar` (registries.json/blockstates) + Bedrock block palette from a current world's subchunk data or extracted Bedrock data; hand-mapped semantic table (state ↔ bedrock name+states) stored as versioned RON — **the RON schema exists now** (`omt-convert::rules`: `BlockRule{name, states}` + `biome_ids` + `block_palette_version`, seed included). `omt-convert`: rayon per-chunk pipeline `read → parse → remap → serialize` (UMT's task stages Load/Parse/Prepare/Convert/Clean minus Light/Render). **As built 2026-11-04:** the pipeline IS that rayon stage chain (`pipeline.rs`), verified end-to-end against a fixture world copy (`java_to_bedrock_fixture.rs`); the registry *generator* and the semantic state table remain the blocked half of M3.
- **[RE]** UMT's DB schema = `LookupEntry{edition,category,id,meta,version,variant}` + `ConversionEntry` (report §5.3, guide §5.2) — **keep the schema** (extensions depend on it), change the *contents source* to our generator.
- **[REA]** ✅ resolved 2026-11-03 (see [`UMT_Converter_RE_Findings.md`](../RE/UMT_Converter_RE_Findings.md) Tasks B–D): `skipProtoChunks` = default **true**, era-1 predicate `Status ∈ {base, empty, structure_starts}` (via `Level`), era-2 keeps only root `Status == full` (after stripping optional `minecraft:` prefix), drop happens at parse (entities/BEs go with the chunk). Entity path: universal lowercase `entities` list ⇄ Java entity-region root `{DataVersion, Position: [I; x, z] (exactly 2 ints), Entities}`; UMT's border-chunk deferral (`0x459C10` + state machine `0x45CC30`) is an artifact of its region-fragmented pipeline — OMT (global addressing) does **not** need it. Subchunk index order **XZY verified** (`getBlock 0x4A9DF0`: `idx = a<<8 | b<<4 | (y&0xF)`).
- **Accept:** convert provided Java fixture → Bedrock, open in Bedrock Win/Android, walk the same area, structures/containers intact; reverse direction ditto. >95% block match on automated diff of a small validation plot.

### M4 — Entities, items, tile entities, level.dat
- **[F]** Entity conversion (id mapping, Pos/Rotation/Motion, variants, age, health, potions, custom names), tile entities (chests/barrels/signs/furnaces: item stacks id⇄runtime-id, count, damage, enchants, book text), item frames/minecarts/boats, `level.dat` field mapping (spawn, difficulty, gamemode, time, gamerules where expressible), inventory/ender-chest NBT editing already via M2.
- **[RE]** UMT: "every entity with properties", `Position` int-list handling `0x459C10`; `_umt_host_chest` = console-only (ignore).
- **[REA]** ✅ resolved 2026-11-03 (findings doc Task A): UMT's item/tag conversion is **fully data-driven** — a runtime-downloaded `database.ndb` (obfuscated `gnXig` store, `/APP/database/`, api.universalminecrafttool.com) holds all key mappings; the binary contains **zero** literals for `Count`/`Slot`/`Damage`/`Components`/`WasPickedUp`/`enchantments`. Engine = generic compound walker (`0x470840`, skips `_`-prefixed internal nodes) + per-item rule interpreter (`0x462920`) driven by a `_tagModifiers` rule column (schema name recovered from `.rdata` 0x5f8398–0x5f8530). OMT therefore builds its **own** rule schema (registry-gen output): per-item `{rename, coerce, drop, default}` ops over `tag`/`components` paths, applied as opaque-passthrough + targeted transforms, every unmatched component logged to the §5C loss ledger. UMT's `converter/general/options/mixedComp` (recovered) evidences that mixed `tag`+`components` worlds need an explicit compat toggle — carry one.
  - **As built 2026-11-04:** the rule engine's first cut lives in `omt-convert::items` (`java_stack_to_bedrock`/`bedrock_stack_to_java` + `ConversionReport`) with the `mixedComp`-equivalent `MixedEraPolicy` and a pluggable `EnchantIdMap` (empty until registry-gen exists — unmapped ids are explicit losses, never guesses).
- **Accept:** chest with named enchanted books + sign text round-trips both directions.

### M5 — Pruner
- **[F]** Layers: Box, Circle, Polygon, Split, Time (InhabitedTime), Void (keep chunk, clear blocks), Optimize (Bedrock compaction only). Patterns Keep/Delete × Inverse, unlimited stack, 2D interactive map with control points + rulers, non-destructive (workspace copy), per-layer enable/drag-reorder, chunk-count preview before apply.
- **[FN]** `SelectionRasterizer` (chunk-space shapes), prune executor (region rewrite for Java — drop dead sectors + rewrite headers; LevelDB delete keys + `digp` cleanup for Bedrock; entity-region trimming).
- **[RE]** UMT layer classes + thumbnails (report §3.4), `InhabitedTime` referenced by 16 functions (chunk-type accessors).
- **[REA]** optional: `PrunerLayerTime` vtable methods — symbols `PrunerLayer*::vftable` (already dumped in evidence) for exact inclusive/exclusive threshold semantics. *2026-11-03 partial:* UI label `"Select if InhabitedTime is:"` + `inhabitedTime` recovered; vtable methods `0x59d3c0/0x59d300/0x59d360/0x4d63e0` are ctor/dtor plumbing — comparator still unfound, stays on the backlog.
- **Accept:** prune a copied fixture down 50% by polygon+time layer; world opens in-game; Bedrock db size shrinks after Optimize.

### M6 — Rendering: Chunk Locator + Slicer
- **[F]** 2D chunk map (pan/zoom), top-render per chunk from block models (not flat colors), biome tint, water/height shading, block lookup by coordinate (click → block id + state), goto-coordinate, Slicer (hide above Y, DualSlider), dimension switcher, generated world thumbnails.
- **[FN]** CPU tile renderer (rayon) → LRU tile cache; resource-pack loader (vanilla assets from user's game dir or pinned client.jar; texture regex from guide §8.2); model-JSON parser (elements/faces/tintindex/display vocabulary — full list recovered).
- **[RE]** UMT renderer classes + uniform names + model vocabulary (guide §8); `BlockTopRenderGenerator`.
- **[REA]** optional: biome tint application (colormap sampling) — `BlockTopRenderGenerator` vtable if colors look wrong vs UMT screenshots.
- **Accept:** locator renders both fixtures at interactive FPS; slicer + click-identify work.

### M7 — LevelDB browser + extension system
- **[F (ldb)]** raw key/value table, async substring/hex search, open value as NBT (chunk keys auto-decoded), hex view.
- **[F (ext)]** mlua host implementing the recovered API verbatim (guide §7): `LuaChunkAccess`, `LuaDatabaseAccess.find` (over our registry), `LuaNBTParser`, `JSONValue`, globals `BASE_PATH/TYPE/EXTENSION_TYPE/BLOCK_FORMAT/FILE_TYPE/EDITION`; zip install (`manifest.json` + `main.lua` + `icon.png`), enable/disable/order; NBT-editor style extensions first-class (bootstrap script already recovered — reuse logic).
- **[RE]** full API surface + bootstrap script in evidence; hub: replace GitHub catalog with our own repo URL (runtime-assembled anyway).
- **[REA]** any behavior gap: registration functions `0x57B7F0` (ConversionContext metatable), `0x57DAE0` (TYPE_MT), name-table blob at file offset `0x4B2468`.
- **Accept:** a community UMT NBT-style extension runs unmodified; ldb browser finds a player key and opens it as NBT.

### M8 — GUI polish + release
- **[F]** Full theme per guide §3.1 tokens; frameless custom title bar; workspaces tabs; settings TOML (guide §10 keys minus account bits); crash-safe workspace recovery; CLI parity for everything (`omt convert/prune/nbt/inspect/ldb`); v0.1 tag + builds (Windows installer, Linux tar/AppImage).

### Standing: quality gates
- `cargo fuzz` targets: NBT reader, region reader, subchunk codec, leveldb key codec.
- Property tests: NBT round-trip BE/LE; region rewrite preserves untouched chunks (byte-compare sectors).
- Golden corpus: §8 fixtures; conversion diff harness compares block arrays pre/post.

---

## 5. UI plan (condensed from guide §3 — latest-only tool, no wizard)

Screens: **Home** (world list + open) → **Workspace** per world with sidebar: `NBT Editor · Converter · Pruner · Locator · LevelDB (Bedrock) · Extensions · Settings`. Converter drops UMT's Step1–4 wizard for a single pane (source world → target edition+version fixed to registry default → options switchers → Convert). Theme tokens copied verbatim from the QSS table (guide §3.1). GUI is **Iced** (0.14, user-mandated — swapped from the original egui plan); core stays CLI-first.

**As built (M2):** Home (world scan + open files via rfd) + tabbed NBT-editor documents with dirty indicators, search, change highlighting, and Ctrl+Z/Y/S/F. The world Workspace screen with the full sidebar module list grows per milestone (Converter pane M3, Pruner M5, Locator M6, ldb browser M7).

## 6. What we reuse from UMT vs. redesign

| Keep (proven by RE) | Redesign (simplify) |
|---|---|
| Virtual path namespace over stores | Format-version ladders → two-constant registry (§2) |
| Task-per-stage chunk pipeline | QThread → rayon |
| Copy-on-open workspace + `_backup` | Wizard UI → single pane |
| Registry schema (LookupEntry/ConversionEntry) | Registry *contents* generated, not hand-curated |
| Pruner layer model + patterns | Console codecs — deleted |
| Undo command pattern, corruption-lenient region IO | DRM/accounts/updater — deleted |
| UMT Lua API surface | — |

## 7. Risks & mitigations

| Risk | Mitigation | Status |
|---|---|---|
| Java 1.20.5+ **item components** format churn | Pin to registry; ~~M4 gated on REA pass~~ **REA pass done 2026-11-03**: UMT itself converts items via downloaded DB rules (`database.ndb` `_tagModifiers`), no binary literals — OMT ships its own rule schema generated from client.jar/Bedrock data; every unmatched component becomes an explicit §5C loss-ledger row. Mixed-era (`tag`+`components`) worlds get a compat toggle (UMT `mixedComp` precedent). | sharpened (M4 design unblocked) |
| Bedrock runtime-id instability across releases | Runtime IDs are per-save palette-referenced — always resolve through each world's own subchunk palettes, never globally; registry maps *names+states*, not runtime ids. | open (principle already followed by M1 decode) |
| ~~leveldb crate lacks raw-zlib blocks~~ | ~~rusty-leveldb supports custom compressors; if not, vendor it (MIT)~~ | ✅ retired — `bedrock-leveldb 0.3.1` reads/writes raw-zlib Bedrock dbs natively (M1) |
| Registry hand-mapping size (~1000 blocks × 2) | Generate 90% from data (Java blockstates ↔ Bedrock block NBT from a probe world), hand-map the semantic 10% (waterlogged, states that don't exist cross-edition). This is the single biggest content task — schedule it as its own workstream inside M3. Note: fixtures carry behavior-pack blocks (`panascais_caves:*`) — best-effort passthrough per §1.2. | open (M3) |
| Mojang API deprecation (session server) | Cache-first, optional feature, degrade gracefully to UUIDs. | open (not yet wired) |
| UMT name proximity (OMT vs UMT) | Intentional homage; README disclaimer keeps the clean-room line explicit (independent implementation, not a fork, not affiliated). | standing |
| ~~Bedrock block index order assumed XZY~~ | ~~Verify against REA `0x4C0F60`~~ | ✅ retired 2026-11-03 — REA verified `getBlock`/`setBlock` (`0x4A9DF0`/`0x4AE610`): `idx = (x<<8)|(z<<4)|y` (Y fastest); UMT (Bedrock ≤26.50) and our 1.26.52 fixtures agree |
| Java **heightmap column order** (z-major assumed) | data3D side empirically verified z-major (99.6%, probe); Java side assumed same order + same +1 encoding (numeric values then copy 1:1). Wrong order = transposed heightmap — single-function fix. | open — verify when Java fixture lands (converter warns via loss rows only if absent) |
| Universal-model order mixing (Java YZX vs Bedrock XZY) | ~~latent: Bedrock reads copied XZY into YZX-typed `Slice`~~ ✅ fixed 2026-11-04 — normalized XZY→YZX at the store boundary; permutation round-trip + absolute-position tests in `omt_world::packing`; e2e fixture test asserts through both permutations |
| Java **pre-1.16 continuous packing** in the old unpacker | ~~latent: `unpack_java_indices` read the pre-1.16 continuous stream~~ ✅ fixed 2026-11-04 — rewritten non-spanning per wiki.vg in `omt_world::packing` (our read floor is 1.18, where non-spanning is the layout); known-vector tests pin the long-boundary behavior |

## 8. Test-world fixtures (what to provide, when)

User-provided; drop into `tools/world-fixtures/` (never commit publicly). **Status:** three Bedrock 1.26.52 worlds provided and in use at `world_ref/` (`e1j7l0I-l68=` "Base", `eCs-bHSnfhQ=` "Alt Settings", `jQS+FzM7v-0=` "Exp" — behavior-pack content included). **Java fixture still needed (gates M3–M6 acceptance).**

| Fixture | Requirements | Needed by |
|---|---|---|
| `java-latest.vanilla` | Fresh survival world, current release; ~10 chunks explored; a village/monument; chests w/ enchanted+named items+books; signs; maps; one of each: furnace, barrel, item frame, boat/minecart; all 3 dims entered | **M3–M6 (missing)** |
| `bedrock-win.latest` | Windows Bedrock world, same content checklist as above | ✅ provided (×3, at `world_ref/`) |
| `bedrock-android.latest` | Android world (copy `minecraftWorlds/<id>` via file manager/adb), same checklist | M1 parity (optional now — folder layout proven identical) |
| `*.mcworld` export of `bedrock-win.latest` | template + world variants | M1 archive open (TODO) |
| Edge: corrupted region | One `.mca` with duplicate sector entries (we can synthesize) | ✅ synthesized in omt-java tests |
| Edge: huge world | Any 1–2 GB world for perf baselines | M5/M6 |

## 9. Milestone order & rough sizes

M0 nbt (S) ✅ → M1 world-open (L) ✅ → M2 nbt-editor (M) ✅ → **M3 registry+converter-core (XL) ← next** → M4 entities/items (L) → M5 pruner (M) → M6 render (L) → M7 ldb+ext (M) → M8 polish/release (M). CLI ships every milestone; GUI became usable at M2, good at M6.

## 10. REA workflow for implementation-time questions

- Snapshot already cached: `/tmp/umt_re/rea_snap.zip` (CLI: `rea decompile "<exe path>" <addr> --snapshot /tmp/umt_re/rea_snap.zip --format json --filter-output raw_result`). **2026-11-03:** `/opt/ghidra` + system JDK were wiped from the host; toolchain restored to `~/Tools/ghidra-inst/ghidra_12.1.2_PUBLIC` + `~/Tools/jdk-21.0.13+9`. Run the CLI with `GHIDRA_INSTALL_DIR`, `JAVA_HOME`, `PATH` exported per invocation (the REA MCP server inherits stale env after host changes).
- After a Zcode restart the REA MCP tools attach directly (registration is aligned) — prefer `open_binary`/`analyze_function` **if** the provider env is healthy (`rea doctor` / first call errors otherwise).
- Ghidra project with full symbols: `/tmp/umt_re/ghidra_proj` (19,141 functions; `functions.txt`/`symbols.txt` in evidence — persists per-reboot; regenerate from `umt_re_evidence/tools/` if lost).
- Capstone/pefile quick passes (string VA → `.text` imm-ref scans) often answer xref questions in seconds without Ghidra — see `Source/RE/UMT_Converter_RE_Findings.md` §Toolchain.
- Always answer unknowns against the binary, not memory: the §4 [REA] entries are the pre-mapped entry points.
- Converter pass entry points (2026-11-03): factory `0x4C0F60`; ChunkTaskParse `0x45AC30` (proto filter); ChunkTaskConvert `0x459C10`; entity-region writer helpers `0x459B10/0x459910/0x459310/0x4596F0`; Bedrock universal chunk class ctor `0x4AA9D0` (vtable `0x5FCED0`), lazy Section decode `0x4A96F0`/`0x4A9000`, getBlock `0x4A9DF0`, setBlock `0x4AE610`; item rule engine: walker `0x470840`, `_tagModifiers` dispatch `0x463940`, interpreter `0x462920`, DB special-cases `0x471190`; options defaults `0x46C1A0`, switcher slot `0x538330`.
