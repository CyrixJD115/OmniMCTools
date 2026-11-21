# The OMT extension platform

> Prefer compiled **Rust** over Lua? Native plugins (dynamic libraries) add the same tools / actions with the full language — see [`crates/omt-plugin/PLUGINS.md`](../omt-plugin/PLUGINS.md).

Extensions are Lua 5.4 packages (`manifest.json` + `main.lua`, installed from
**Extensions → Install from .zip…**). `main.lua` returns a table of entries;
each entry's `type` says what it adds:

| `type` | Adds | Runs |
|---|---|---|
| `EXTENSION_TYPE.NBT_EDITOR_STYLE` | row annotations / colours / edits in the NBT editor | every pass over a document (see `crates/omt-script/EXTENSIONS.md`) |
| `EXTENSION_TYPE.TOOL` | a **tab of its own** in the sidebar (and, with `map = true`, a **Map Viewer panel**) with your own UI | its own worker thread, alive while the tab is open |
| `EXTENSION_TYPE.NBT_ACTION` | an **action button** on the NBT editor's selected tag | like a tool, opened on that tag |

Everything below is about tools and actions. Extensions run with full trust
(full Lua standard library) — install only what you trust. A broken callback
never takes the app down: its error is shown in the tool and the tool keeps
running. Long callbacks never freeze the app (worker thread); the **Stop**
button cancels them.

The built-in example (**Extensions → Install examples**, source in
`assets/chest-explorer/main.lua`) uses everything here: a container finder
with search and chest menus, player inventories, and an "Open as chest" NBT
action.

## Writing your own

1. **Extensions → New extension…** creates `my-extension/` (a working tool
   with a world summary, a button, an input, storage and map clicks), turns
   it on, and opens its folder. Its tool appears in the sidebar at once.
2. Edit `main.lua` in any editor.
3. Press **Reload** on the tool's tab — it restarts from the files on disk,
   on the same world (or NBT tag). **Reload all** on the Extensions page
   re-reads every extension (new tools / actions, renamed entries).
4. Errors show at the top of the tool with the Lua line number; `print` /
   `omt.log` output shows under it.
5. Share it: zip the folder (`manifest.json` + `main.lua` at the zip root, or
   in one top-level folder) — others install it with **Install from .zip…**.

## Built-in extensions and the master switch

OMT ships extensions embedded in the binary (`omt_extensions::builtins()`):
**Chest Explorer** and **Tick Clock**. They are listed on the Extensions page
even before they are installed; enabling one writes its embedded files into
the extensions folder (so they stay inspectable and editable) and turns it
on — nothing is written for players who never opt in, and Remove brings the
card back as "not installed". In addition, **Settings → Extensions** (or the
switch at the top of the Extensions page) turns the whole system off: no
tool tabs, no NBT actions, no row notes, no discovery — per-extension
toggles are remembered and apply again when it is turned back on.

## A tool

```lua
local Tool = {
  type = EXTENSION_TYPE.TOOL,
  id = "hello",                 -- unique inside your extension
  name = "Hello",               -- sidebar label / tab title
  icon = "box",                 -- see "Icons"
  description = "Says hello.",
  map = true,                   -- also offer it in the Map Viewer panel
}

function Tool:init(ctx)         -- once, when the tab opens
  self.count = 0
end

function Tool:on_event(ev, ctx) -- every click / input / map click
  if ev.kind == "click" and ev.id == "more" then self.count = self.count + 1 end
end

function Tool:view(ctx)         -- after init and after every event
  return ui.column{
    ui.heading("Hello " .. (ctx.world and ctx.world.name or "nobody")),
    ui.button{ id = "more", label = "Clicked " .. self.count .. "×", style = "primary" },
  }
end

return { Tool }
```

* Keep your state on `self` — the same table lives as long as the tab.
* `view` returns a tree of `ui.*` nodes; it is called after every callback,
  so the screen always shows your current state.
* `ctx.world` is the world of the session the tool was opened from (the one
  on screen, or the last one you looked at) — `nil` when none is open.
  `ctx.edition` is `"JAVA"` / `"BEDROCK"`.
* `self.title = "…"` changes the tab title.

## UI nodes

Every constructor takes a table; children go in the array part (or
`children = {…}`). `nil` children are skipped, so
`cond and ui.text("…") or nil` works. A plain string child is a `ui.text`.

| Node | Fields | Events |
|---|---|---|
| `ui.column{…}` / `ui.row{…}` | `spacing` | — |
| `ui.card{ title = …, … }` | titled panel | — |
| `ui.scroll{ height = 300, … }` | scrollable area | — |
| `ui.text{ text, size, color = "#rrggbb" or {r,g,b}, bold, mono }` / `ui.text("…")` | | — |
| `ui.heading("…")`, `ui.separator{}`, `ui.space{ size }` | | — |
| `ui.code("…")` | monospace block (SNBT, reports) | — |
| `ui.button{ id, label, style = "primary"/"danger"/"ghost", icon, enabled, pick }` | `pick = "file"/"folder"/"save"/"world"` asks for a path first | `click`, or `picked` with `ev.path` |
| `ui.input{ id, value, placeholder, width }` | typing is shown at once | `input` (every key), `submit` (Enter) — `ev.value` |
| `ui.toggle{ id, label, value }` | | `toggle` — `ev.value` |
| `ui.slider{ id, label, min, max, step, value }` | | `slider` on release — `ev.value` |
| `ui.select{ id, options = {…}, value, placeholder }` | | `select` — `ev.value` |
| `ui.table{ id, columns = {…}, rows = {{…}, …}, selected, height }` | rows are 1-based | `row` — `ev.row` |
| `ui.chest{ id, title, rows, cols, items, selected }` | a Minecraft container with item icons, counts, glint and tooltips; `items` as returned by `omt.items.read` / `world:containers` (or `{ slot, id, count, label, lore, glint }`) | `slot` — `ev.slot` |
| `ui.block{ name, size }` | a block icon from the active textures | — |
| `ui.progress{ value = 0..1, label }` | | — |

Every event is `{ kind = …, id = <node id>, … }`. Map Viewer clicks (tools
with `map = true`, panel open) arrive as `{ kind = "map_click", x, z, dim }`
(block coordinates; `dim` 0/1/2).

**Icons** (`icon =` on tools and buttons): `search box chest map player
spawn target clock filter list grid refresh save export info eye layers leaf
palette sparkles compass sun texture entity potion cave plus check close`.

## NBT actions

```lua
local Action = {
  type = EXTENSION_TYPE.NBT_ACTION,
  id = "open-as-chest", name = "Open as chest",
  names = { "Items", "Inventory" },   -- tag names it applies to (omit = any)
  types = { TYPE.LIST },              -- tag types it applies to (omit = any)
}
function Action:view(ctx) return ui.chest{ id = "c", items = omt.items.read(ctx.node) } end
return { Action }
```

The action shows as a button in the NBT editor's *Selected tag* panel when
the selected tag matches. It opens like a tool with `ctx.node` = that tag
(a tag handle, full tag API). **Edit `ctx.node` and the editor receives the
change as one undoable step** (Ctrl+Z reverts it).

## The `omt` library

### Worlds — `omt.world`

| Call | Returns |
|---|---|
| `omt.world.current()` | the tool's world (same as `ctx.world`) or `nil` |
| `omt.world.open(path)` | another world, opened read-only |
| `omt.world.list()` | the app's library: `{ {name, path, edition}, … }` |

A world `w` has `w.name`, `w.path`, `w.edition` (`"JAVA"`/`"BEDROCK"`),
`w.writable`, and:

| Method | Returns |
|---|---|
| `w:level()` | level.dat as a tag (`nil` if unreadable) |
| `w:players()` | `{ {id, tag}, … }` (Java: `playerdata/` and legacy `players/`; Bedrock: the local player) |
| `w:player(id)` | one player tag (Bedrock: `"local"` or a raw player key) |
| `w:chunks(dim)` | `{ {x, z}, … }` chunk coordinates present |
| `w:chunk(dim, cx, cz)` | `{ x, z, data_version, inhabited_time, status, block_entities = {…}, entities = {tag…} }` or `nil` |
| `w:block(dim, x, y, z)` | `name, { property = value }` |
| `w:block_entities(dim, x0, z0, x1, z1, filter)` | `{ {x, y, z, id, kind, tag}, … }`; `filter` = kind substring (`"sign"`) or `function(be) return … end` |
| `w:containers(dim, x0, z0, x1, z1)` | `{ {x, y, z, kind, rows, cols, items, tag}, … }` — chests, barrels, shulkers, hoppers, furnaces, dispensers… |
| `w:entities(dim, x0, z0, x1, z1)` | `{ {id, x, y, z, tag}, … }` |
| `w:set_block_entity(dim, x, y, z, tag)` | writes a block entity back (container items, signs, spawners…) |
| `w:set_block(dim, x, y, z, block)` | sets one block — `block` = `"stone"` or `"oak_log[axis=y]"`; returns whether it changed |
| `w:fill(dim, x0, y0, z0, x1, y1, z1, block, { replace = "dirt" })` | fills a box (optionally only blocks matching `replace`); returns blocks changed (≤ 4 000 000 per call) |

**Writes** work on editable sessions (a world opened in the **NBT Editor**
or **World Editor** — a workspace copy) and go through the same per-edition
writers as the World Editor (Bedrock LevelDB records, Java region files,
atomic). The session is marked modified; its **Save** puts the changes into
the real world, closing without saving discards them. Block entities at
blocks a fill changes are removed, as in the game. Map Viewer sessions are
read-only (writes raise a clear error; check `w.writable`). While the
session saves, its open tools are paused and then restarted on the saved
world (they run `init` again — keep lasting state in `omt.storage`).

```lua
-- Add 5 diamonds to the nearest chest, place a gold block, in an editable session.
local c = ctx.world:containers(0, -64, -64, 64, 64)[1]
omt.items.put(c.tag, omt.items.make{ id = "diamond", count = 5, slot = 4 })
ctx.world:set_block_entity(0, c.x, c.y, c.z, c.tag)
ctx.world:set_block(0, 0, 100, 0, "gold_block")
```

`dim`: 0 Overworld, 1 Nether, 2 End. Rectangles are block coordinates;
long scans report progress and stop when the user presses **Stop**. Queries
return at most 20 000 results.

### NBT — `omt.nbt`

| Call | |
|---|---|
| `omt.nbt.read(path)` | `tag, format` — any Java/Bedrock NBT file (format auto-detected) |
| `omt.nbt.write(path, tag, format)` | `format` = `"java"` (gzip, default), `"java_raw"`, `"bedrock"` (raw LE), `"bedrock_level"` (level.dat header) |
| `omt.nbt.parse(snbt)` / `omt.nbt.snbt(tag, pretty)` | SNBT in / out |
| `omt.nbt.list(values)` | a list tag (`omt.nbt.list()` = empty list) |
| `omt.nbt.new(value)` | a tag from plain Lua: integers → Int, floats → Double, strings, booleans → Byte, arrays → List, tables → Compound; `omt.nbt.new()` = empty compound |
| `omt.nbt.is_tag(v)` | |

Tags have the full tag API (`child`, `find`, `getInt`/`setInt`/`addInt`…,
`addChild`, `removeChild`, `clone`, `getCompound`/`setCompound`…) documented
in `crates/omt-script/EXTENSIONS.md`. Tags from different sources mix
freely (`list:addChild(omt.items.make{…})` copies across documents).

### Items — `omt.items`

| Call | |
|---|---|
| `omt.items.read(tag)` | an item list (or a tag holding `Items` / `Inventory` / `EnderItems`) → `{ {slot, id, count, damage, label, lore, enchantments = {{id, level}}, glint, contents = {…}}, … }` — Java 1.20.5+ components, older Java (incl. numeric pre-1.8 ids), Bedrock |
| `omt.items.put(container, item, key)` | put an item tag into a container / inventory tag (`key` defaults to `"Items"`): creates the list when missing (empty Java chests have none), replaces a stack already in that slot |
| `omt.items.make{ id, count, slot, edition }` | a new item tag in that edition's layout (default: the world's) |
| `omt.items.layout(kind)` | `rows, cols` of a container kind (`nil` if not a container) |
| `omt.items.kind(id)` | normalized container kind (`ShulkerBox` → `shulker_box`) |
| `omt.items.name(id)` | `minecraft:fire_aspect` → `Fire Aspect` |

### Everything else

| Call | |
|---|---|
| `omt.notify(msg)` | status line message |
| `omt.clipboard(text)` | copy text |
| `omt.progress(text, fraction)` | progress line + bar while a callback runs |
| `omt.cancelled()` | the user pressed Stop (loops can bail out early) |
| `omt.log(…)`, `print(…)` | tool log (shown under the tool) |
| `omt.json.encode(v, pretty)` / `omt.json.decode(s)` | JSON |
| `omt.storage.get(k)` / `set(k, v)` / `all()` | data saved per extension (`storage.json` in your folder), kept across sessions |
| `omt.version` | app version |

## Tips

* Heavy work (whole-world scans) belongs in `on_event` / `init`, not
  `view` — `view` runs after every event.
* Use `omt.progress` inside long loops and check `omt.cancelled()`.
* Test with the debug hooks: `OMT_DEBUG_OPEN_TOOL=<ext>/<tool>` (plus
  `OMT_DEBUG_TOOL_EVENTS="submit:search:diamond;slot:chest:0;map:120,64"`,
  `OMT_DEBUG_TOOL_PANEL=1`) and `OMT_DEBUG_NBT_ACTION="<tag name>=<ext>/<id>"`.
