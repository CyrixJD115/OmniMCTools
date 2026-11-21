# OMT extensions (Lua)

> **Tools, NBT actions, Map Viewer panels and the `omt` library (worlds,
> NBT files, items, JSON, storage…)** are documented in
> [`crates/omt-extensions/EXTENSIONS.md`](../omt-extensions/EXTENSIONS.md).
> This page covers the package format, the NBT-editor style protocol and the
> tag API they all share.

OMT's extension system runs Lua 5.4 packages that decorate the NBT editor —
extra annotations on rows, descriptions, colors, and (careful) tree edits.
The protocol mirrors the behavior of UMT's extension bootstrap so
NBT-editor-style extensions port with minimal changes; everything UMT never
exposed is an OMT addition and marked as such below.

- **Trust model:** extensions are plain Lua with the full standard library.
  Install only from sources you trust. The one hardening measure: loading
  native C modules (`package.cpath`) is disabled.
- **Runtime:** one fresh Lua VM per pass over a document. Globals are reset
  every run; keep state in files under `BASE_PATH` if you need any.

## Package layout

A zip (or folder) with `manifest.json` and `main.lua` at the root — or under
a single top-level folder inside the zip:

```
tick-clock.zip
├── manifest.json
├── main.lua
└── lib/…        (optional; require("tick-clock.lib…") works via BASE_PATH)
```

`manifest.json` (unknown keys ignored):

```json
{
  "name": "Tick Clock",        // required; slugified to the install id
  "version": "1.0.0",
  "author": "you",
  "description": "shows tick counters as in-game time"
}
```

Install zips from the GUI's **🧩 Extensions** page; the built-in example
(`crates/omt-script/assets/ticks-time/`) can be installed with one click.

## The protocol

`main.lua` **returns a table of sub-extensions**. Every entry with
`type = EXTENSION_TYPE.NBT_EDITOR_STYLE` is registered. For each document the
host then:

1. calls every sub-extension's `main(self, root, context)` **once**, in
   registration order, then
2. walks the tree **depth-first** (preorder: node, then children of
   compounds and lists via `child(0) … child(childCount-1)`), calling each
   recursive sub-extension's `recursion(self, root, current, context)` at
   every node — extensions in registration order at each node.

```lua
local E = {}
E.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
E.recursive = true          -- omit (or false) for main-only extensions

function E:main(root, context) end
function E:recursion(root, current, context)
  if current.type == TYPE.LONG and current.name == "Time" then
    current:setExtraText("day " .. math.floor(current.value / 24000))
  end
end

return { E }                -- several sub-extensions per file is fine
```

Errors in any callback are collected and shown in the editor status line;
they never abort the walk or the editor.

## Globals

| Global | Meaning |
|---|---|
| `BASE_PATH` | directory containing all installed extensions (yours: `BASE_PATH/<id>/`) |
| `TYPE` | NBT type ids: `UNDEFINED BYTE SHORT INT LONG FLOAT DOUBLE BYTE_ARRAY STRING LIST COMPOUND INT_ARRAY LONG_ARRAY` (0–12) |
| `EXTENSION_TYPE` | `{ NONE, NBT_EDITOR_STYLE, TOOL, NBT_ACTION }` |
| `FILE_TYPE` | bit flags: `CHUNK MAP PLAYER LEVEL GENERIC` — test with `context.type & FILE_TYPE.LEVEL` |
| `EDITION` | `{ JAVA, BEDROCK }` (numeric) |
| `logger` | `logger:info/warn/error(msg)` → the run log (also shown in the editor status) |
| `print` | routed into the run log (not stdout) |
| `require` | resolves `BASE_PATH/?.lua`, so `require("<id>.lib.util")` works |
| `context` | `{ type = <FILE_TYPE bits>, edition = "JAVA"|"BEDROCK"|"UNKNOWN" }` |

`FILE_TYPE`/`EDITION` numeric values are OMT-defined; always compare against
the tables, never hard-code numbers.

## Tag userdata

`root` and `current` are tag handles. **Properties:**

| Property | Value |
|---|---|
| `name` | compound key; `""` for list items and the root |
| `value` | scalar value (number/string/array table); `nil` for containers |
| `type` | `TYPE` id (root is `TYPE.COMPOUND` in practice) |
| `parentTag` | parent handle or `nil` |
| `childCount` | children of compounds/lists; 0 otherwise |
| `listType` | element `TYPE` id for lists (0 when empty); 0 otherwise |

**Structure & identity:** `hasParent()`, `getRow()` (0-based position in
parent; −1 for root), `clone()` (detached deep copy), `child(i)` (0-based,
`nil` when out of range), `contains(keyOrIndex)`, `find(name)`.

**Styling (OMT-defined):** `setExtraText(s)`, `setDescription(s)`,
`setColor(c)` where `c` is `"#rrggbb"`, `{r, g, b}` (0–255) or `0xRRGGBB`.

**Typed access:** `get*/set*/add*` for
`Byte Bool Short Int Long Float Double String ByteArray IntArray LongArray
Compound List`:

- `t:getX(sel?)` — with a selector (`"key"` on compounds, 0-based index on
  lists) reads that child; without it reads the tag itself. Returns `nil`
  when missing or wrongly typed.
- `t:setX(value)` sets the tag itself; `t:setX(sel, value)` sets the child.
  The target must already be that type — widths are part of NBT identity.
- `t:addX(value)` appends to a list; `t:addX(name, value)` inserts into a
  compound. List element types must stay homogeneous (typed error otherwise).
- `getCompound`/`getList`/`setCompound`/`setList`/`addCompound`/`addList`
  move subtrees; `setCompound(src)` / `setCompound(sel, src)` replace this
  (or the selected) node's subtree with a copy of `src`'s subtree.
- `addChild(tag)` appends a copy of `tag`; `removeChild(keyOrIndex)`,
  `clear()`, `getValue()`, `setValue(v)` (type-preserving), `getSize()`
  (big-endian serialized payload bytes).

## Edits and write-back

Tree edits made through the setters/adders/removers are written back through
the editor's **undo engine as one step** — the user sees *"an extension
modified the document — Ctrl+Z reverts"*. At most one extension edit is
applied per tree state: **undoing an extension's edit is respected** (the
same tree is never re-edited), and any further change you make re-arms the
pass for one more (convergent) edit. Extensions re-run automatically whenever
the document or the extension set changes, so **write idempotent callbacks**:
after your edit, a second pass over the edited tree should produce no further
change. A non-convergent extension that keeps editing an already-edited tree
is skipped with a status note (no loops).

## Limits

- The walk visits at most 20 000 nodes per pass (status note when truncated).
- Styling keys map to the *original* row paths; tags created during the run
  have no row identity and cannot be styled.
- Lua `==` compares handles, not nodes.
- Registration order of multiple sub-extensions inside one `main.lua` follows
  Lua `pairs` order (unspecified), as in UMT. Order *across* packages is the
  order shown on the Extensions page.

## The example

See `assets/ticks-time/main.lua` (this crate) — 60 lines that annotate
`Time` / `DayTime` / `InhabitedTime` with in-game clock and play-time text,
using `context.type`, typed checks, and all three styling calls.
