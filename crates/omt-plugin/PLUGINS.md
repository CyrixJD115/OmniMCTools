# Native plugins for OmniMCTools

OMT extensions can be written two ways:

| | Lua extension | **Native plugin** |
|---|---|---|
| Language | Lua 5.4 | **Rust** (compiled) |
| Form | `main.lua` in a folder | a dynamic library (`.so` / `.dll` / `.dylib`) in a folder |
| Speed / power | great for most tools | the full language: any crate, real types, heavy computation, threads |
| Guide | [`crates/omt-script/EXTENSIONS.md`](../omt-script/EXTENSIONS.md) and [`crates/omt-extensions/EXTENSIONS.md`](../omt-extensions/EXTENSIONS.md) | this page |

Both add the **same things** — tool tabs, Map Viewer panels and NBT editor
actions — and use the **same `omt` host calls** (worlds, NBT, items,
storage). A native plugin is for when Lua isn't enough: you want crates from
crates.io, strong typing, or performance.

> Native plugins run as native code in the OMT process, with the full trust
> of any program you run. **Install only plugins you trust and built, or
> whose source you have read.** A panic inside a plugin is caught and shown
> as an error (it won't crash OMT), but a native plugin is not sandboxed.

## 1. Create the crate

A native plugin is a `cdylib` that depends on the `omt-plugin` SDK:

```toml
# Cargo.toml
[package]
name = "my-plugin"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
omt-plugin = { git = "https://github.com/CyrixJD115/OmniMCTools" }   # or a path

# The SDK implements the C ABI, so your crate opts out of forbid-unsafe:
[lints.rust]
unsafe_code = "allow"
```

The complete, working example is [`examples/native-sample`](../../examples/native-sample/src/lib.rs).

## 2. Write a tool

Implement [`Instance`](src/lib.rs) — `view` builds the UI, `on_event`
handles input. The instance persists across events for its open tab; keep
your state on `self`.

```rust
use omt_plugin::prelude::*;

struct Hello { clicks: u32 }

impl Instance for Hello {
    fn view(&mut self, _host: &Host, ctx: &Ctx) -> Node {
        let who = ctx.world().map_or("nobody", |w| &w.name);
        ui::column([
            ui::heading(format!("Hello, {who}")),
            ui::button("go", format!("Clicked {} times", self.clicks)).style(ButtonStyle::Primary).into(),
        ])
    }
    fn on_event(&mut self, ev: &Event, host: &Host, _ctx: &Ctx) {
        if let Event::Click { id } = ev {
            if id == "go" { self.clicks += 1; host.notify("hi!"); }
        }
    }
}

fn plugin() -> Plugin {
    Plugin::new("My Plugin").version("1.0.0").author("you")
        .tool(
            ToolDef::new("hello", "Hello").icon("sparkles").map(true).describe("Says hello."),
            |_ctx| Box::new(Hello { clicks: 0 }),
        )
}

export_plugin!(plugin);
```

`export_plugin!` writes the C ABI. That's the only "magic"; everything else
is ordinary Rust.

## 3. Build and install

```text
cargo build --release
```

Then in OMT: **Extensions → Install plugin…** and pick the built library
(`target/release/libmy_plugin.so` on Linux), or drop the library into a new
folder inside OMT's extensions directory. Enable it — its tool appears in the
sidebar (and, with `.map(true)`, in the Map Viewer's *Extension tools*
panel). **Reload** on the tool's tab restarts it from the rebuilt library,
so the loop is: edit → `cargo build` → Reload.

## UI nodes (`ui::`)

`column` `row` `card(title, …)` `scroll(height, child)` `text` `text_styled`
`heading` `separator` `space` `code` · `button(id, label)` (`.style`, `.icon`,
`.pick`, `.enabled`) · `input(id, value, placeholder)` · `toggle` · `slider` ·
`select` · `table(id, columns, rows, selected)` (rows 1-based in the `Row`
event) · `chest(id, title, rows, cols, items)` (a Minecraft container of
`Item`s) · `block(name, size)` · `progress` · `badge(text, color)` · `image(path, size)` · `number(id, value, min, max, step)`.

Events (`on_event`): `Click` `Input` (each key) `Submit` (Enter) `Toggle`
`Slider` `Select` `Row { row }` `Slot { slot }` `Picked { path }` (from a
button with `.pick(Pick::File/Folder/SaveFile/World)`) and, for `map(true)`
tools, `MapClick { x, z, dim }`.

## The host (`&Host`)

Every callback gets a `&Host` that reaches back into OMT. NBT values are
**SNBT strings** (`omt` has no Rust-ABI coupling). Highlights:

| Call | |
|---|---|
| `host.notify(msg)` / `log` / `clipboard` / `progress(text, f)` / `clear_progress` / `cancelled()` | app integration |
| `host.storage_get(key)` / `storage_set(key, json)` | per-plugin saved data |
| `host.world_list()` | the app's worlds |
| `host.containers(dim, x0,z0,x1,z1)` | chests/barrels/shulkers/hoppers… with items + layout |
| `host.block_entities(dim, …, filter)` | block entities by kind substring |
| `host.entities(dim, x0,z0,x1,z1, filter)` | entities (mobs, items…) with position + tag SNBT |
| `host.biome(dim, x,y,z)` / `host.height(dim, x,z)` | biome name / surface Y |
| `host.block(dim, x,y,z)` | `(name, properties)` |
| `host.players()` / `host.level()` / `host.chunks(dim)` | player tags / level.dat / chunk list |
| `host.set_block_entity(dim, x,y,z, snbt)` | write a block entity |
| `host.set_block(dim, x,y,z, "oak_log[axis=y]")` / `host.fill(dim, a, b, block, replace)` | block edits |
| `host.items_read(snbt)` / `host.item_make(id, count, slot, java)` | normalized items |
| `host.nbt_read(path)` / `host.nbt_write(path, snbt, format)` / `host.snbt(s, pretty)` / `host.nbt_check(s)` | NBT files / SNBT |
| `host.replace_node(snbt)` | NBT actions: hand the edited tag back (one undo step) |

**Writes** (`set_block`, `fill`, `set_block_entity`) work only on an editable
session (a world opened in the NBT Editor or World Editor). Check
`ctx.writable()`. The session's **Save** writes them to the real world.
Long scans report progress and stop when `host.cancelled()`.

## NBT actions

An action opens on the NBT editor's selected tag:

```rust
Plugin::new("…").action(
    ActionDef::new("fill-stacks", "Fill stacks to 64")
        .names(&["Items", "Inventory"])   // tag names it applies to (empty = any)
        .types(&[types::LIST]),           // tag types (see omt_plugin::types)
    |_ctx| Box::new(FillStacks),
)
```

`ctx.node()` is the selected tag as SNBT; call `host.replace_node(new_snbt)`
to edit it — the editor applies it as one undoable step.

## How it works (for the curious)

The library boundary is pure C ([`omt_plugin_abi::ffi`]): the plugin exports
`omt_plugin_abi_version`, `omt_plugin_register` and `omt_plugin_dispatch`.
Messages cross as JSON using a **sink** callback — no buffer is allocated by
one side and freed by the other, so there's no shared-allocator hazard and
no reliance on Rust's (unstable) ABI. OMT runs each open tool on its own
thread and services the plugin's host calls synchronously. All `unsafe` is
confined to the SDK's `export_plugin!` glue and OMT's loader; your plugin
code is ordinary safe Rust.

The protocol version is `omt_plugin_abi::ABI_VERSION`; OMT refuses a plugin
built against a different one (rebuild it against the matching OMT).
