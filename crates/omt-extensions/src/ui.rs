//! Declarative UI for extension tools: Lua builds a tree of plain tables
//! with the `ui.*` constructors (a Lua prelude), the host parses it into
//! [`UiNode`] and the GUI renders it with its own widgets (theme, pixel
//! icons, Minecraft-style chest grids). User input comes back as
//! [`ToolEvent`]s, delivered to the tool's `on_event`.

use mlua::{Lua, Table, Value};

/// One item stack in a chest grid.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemSlot {
    /// Slot index inside its container.
    pub slot: i32,
    /// Namespaced item id (`minecraft:diamond`).
    pub id: String,
    pub count: i32,
    /// Custom name / display text (tooltip title when set).
    pub label: Option<String>,
    /// Extra tooltip lines (enchantments, lore, damage…).
    pub lore: Vec<String>,
    /// Draw the enchantment glint.
    pub glint: bool,
}

/// Visual intent of a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonStyle {
    #[default]
    Standard,
    Primary,
    Danger,
    Ghost,
}

/// What a button asks the GUI for before the event fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pick {
    #[default]
    None,
    File,
    Folder,
    SaveFile,
    /// A world from OMT's library (event carries its folder).
    World,
}

/// A node of an extension's UI.
#[derive(Debug, Clone, PartialEq)]
pub enum UiNode {
    Column { children: Vec<UiNode>, spacing: f32 },
    Row { children: Vec<UiNode>, spacing: f32 },
    /// Titled panel.
    Card { title: Option<String>, children: Vec<UiNode> },
    /// Scrollable area (vertical).
    Scroll { child: Box<UiNode>, height: Option<f32> },
    Text { text: String, size: f32, color: Option<(u8, u8, u8)>, bold: bool, mono: bool },
    Heading { text: String },
    Separator,
    Space { size: f32 },
    Button { id: String, label: String, style: ButtonStyle, pick: Pick, enabled: bool, icon: Option<String> },
    Input { id: String, value: String, placeholder: String, width: Option<f32> },
    Toggle { id: String, label: String, value: bool },
    Slider { id: String, min: f64, max: f64, step: f64, value: f64, label: String },
    Select { id: String, options: Vec<String>, value: Option<String>, placeholder: String },
    /// Clickable rows: `rows[i][j]` under `columns[j]`.
    Table { id: String, columns: Vec<String>, rows: Vec<Vec<String>>, selected: Option<usize>, height: Option<f32> },
    /// A Minecraft container: `cols` per row, `rows` rows.
    Chest { id: String, title: String, rows: usize, cols: usize, items: Vec<ItemSlot>, selected: Option<i32> },
    /// A block (or item) icon from the active textures.
    Block { name: String, size: f32 },
    Progress { value: f32, label: String },
    /// Monospace text block (SNBT, logs, reports).
    Code { text: String },
    /// A small coloured pill.
    Badge { text: String, color: Option<(u8, u8, u8)> },
    /// An image file at `size` px.
    Image { path: String, size: f32 },
    /// A numeric field with − / + steppers.
    Number { id: String, value: f64, min: f64, max: f64, step: f64 },
}

/// User input sent to a tool's `on_event(self, ev, ctx)`.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolEvent {
    Click { id: String },
    Input { id: String, value: String },
    Submit { id: String, value: String },
    Toggle { id: String, value: bool },
    Slider { id: String, value: f64 },
    Select { id: String, value: String },
    Row { id: String, row: usize },
    Slot { id: String, slot: i32 },
    /// A button with `pick` got a path (file / folder / world).
    Picked { id: String, path: String },
    /// A click on the Map Viewer while the tool's panel is open.
    MapClick { x: i32, y: Option<i32>, z: i32, dim: u8 },
}

impl ToolEvent {
    /// The event as the Lua table `on_event` receives (`ev.kind`, `ev.id`, …).
    pub(crate) fn to_lua(&self, lua: &Lua) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        let (kind, id) = match self {
            Self::Click { id } => ("click", id),
            Self::Input { id, value } => {
                t.set("value", value.as_str())?;
                ("input", id)
            }
            Self::Submit { id, value } => {
                t.set("value", value.as_str())?;
                ("submit", id)
            }
            Self::Toggle { id, value } => {
                t.set("value", *value)?;
                ("toggle", id)
            }
            Self::Slider { id, value } => {
                t.set("value", *value)?;
                ("slider", id)
            }
            Self::Select { id, value } => {
                t.set("value", value.as_str())?;
                ("select", id)
            }
            Self::Row { id, row } => {
                // Lua-style 1-based row.
                t.set("row", *row as i64 + 1)?;
                ("row", id)
            }
            Self::Slot { id, slot } => {
                t.set("slot", *slot)?;
                ("slot", id)
            }
            Self::Picked { id, path } => {
                t.set("path", path.as_str())?;
                ("picked", id)
            }
            Self::MapClick { x, y, z, dim } => {
                t.set("x", *x)?;
                t.set("y", *y)?;
                t.set("z", *z)?;
                t.set("dim", *dim)?;
                t.set("kind", "map_click")?;
                return Ok(t);
            }
        };
        t.set("kind", kind)?;
        t.set("id", id.as_str())?;
        Ok(t)
    }
}

/// The `ui` library: constructors only tag a table with its `kind`, so
/// authors can also build nodes by hand (`{ kind = "text", text = "hi" }`).
pub(crate) const PRELUDE: &str = r#"
ui = {}
local function node(kind)
  return function(t)
    if type(t) ~= "table" then t = { t } end
    t.kind = kind
    return t
  end
end
for _, k in ipairs({ "column", "row", "card", "scroll", "text", "heading", "separator", "space", "button",
                     "input", "toggle", "slider", "select", "table", "chest", "block", "progress", "code", "badge", "image", "number" }) do
  ui[k] = node(k)
end
-- ui.text("hello") / ui.heading("Title") / ui.code("...") take a plain string too.
for _, k in ipairs({ "text", "heading", "code", "badge" }) do
  local make = ui[k]
  ui[k] = function(t)
    if type(t) ~= "table" then t = { text = tostring(t) } end
    return make(t)
  end
end
"#;

fn s(t: &Table, k: &str) -> String {
    match t.get::<Value>(k) {
        Ok(Value::String(v)) => v.to_string_lossy(),
        Ok(Value::Integer(i)) => i.to_string(),
        Ok(Value::Number(n)) => n.to_string(),
        Ok(Value::Boolean(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn opt_s(t: &Table, k: &str) -> Option<String> {
    Some(s(t, k)).filter(|v| !v.is_empty())
}

fn num(t: &Table, k: &str) -> Option<f64> {
    match t.get::<Value>(k) {
        Ok(Value::Integer(i)) => Some(i as f64),
        Ok(Value::Number(n)) => Some(n),
        _ => None,
    }
}

fn flag(t: &Table, k: &str, default: bool) -> bool {
    match t.get::<Value>(k) {
        Ok(Value::Boolean(b)) => b,
        Ok(Value::Nil) | Err(_) => default,
        Ok(_) => true,
    }
}

/// `{r, g, b}` or `"#rrggbb"`.
fn color(t: &Table, k: &str) -> Option<(u8, u8, u8)> {
    match t.get::<Value>(k).ok()? {
        Value::Table(c) => Some((c.get::<u8>(1).ok()?, c.get::<u8>(2).ok()?, c.get::<u8>(3).ok()?)),
        Value::String(h) => {
            let h = h.to_string_lossy();
            let h = h.trim_start_matches('#');
            let v = u32::from_str_radix(h, 16).ok()?;
            (h.len() == 6).then_some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
        }
        _ => None,
    }
}

/// Children: the array part of the table (and `children = {...}`).
fn children(t: &Table, depth: usize) -> mlua::Result<Vec<UiNode>> {
    let src = match t.get::<Value>("children")? {
        Value::Table(c) => c,
        _ => t.clone(),
    };
    // Integer keys in order, holes skipped (`cond and ui.x{} or nil`).
    let mut items: Vec<(i64, Value)> = src.pairs::<Value, Value>().filter_map(Result::ok).filter_map(|(k, v)| match k {
        Value::Integer(i) if i >= 1 => Some((i, v)),
        _ => None,
    }).collect();
    items.sort_by_key(|(i, _)| *i);
    let mut out = Vec::new();
    for (_, v) in items {
        if let Some(n) = parse_value(&v, depth + 1)? {
            out.push(n);
        }
    }
    Ok(out)
}

/// Parse an item table (`omt.items` output or hand-made).
pub(crate) fn parse_item(t: &Table) -> ItemSlot {
    let lore = match t.get::<Value>("lore") {
        Ok(Value::Table(l)) => l.sequence_values::<String>().filter_map(Result::ok).collect(),
        _ => Vec::new(),
    };
    ItemSlot {
        slot: num(t, "slot").unwrap_or(-1.0) as i32,
        id: s(t, "id"),
        count: num(t, "count").unwrap_or(1.0) as i32,
        label: opt_s(t, "label").or_else(|| opt_s(t, "name")),
        lore,
        glint: flag(t, "glint", false) || flag(t, "enchanted", false),
    }
}

/// Parse a UI value (`nil` / non-node values are skipped).
pub(crate) fn parse_value(v: &Value, depth: usize) -> mlua::Result<Option<UiNode>> {
    match v {
        Value::Table(t) => parse(t, depth).map(Some),
        Value::String(text) => Ok(Some(UiNode::Text { text: text.to_string_lossy(), size: 13.0, color: None, bold: false, mono: false })),
        _ => Ok(None),
    }
}

/// Parse one node table.
pub(crate) fn parse(t: &Table, depth: usize) -> mlua::Result<UiNode> {
    if depth > 64 {
        return Err(mlua::Error::runtime("ui tree nested deeper than 64 levels"));
    }
    let kind = s(t, "kind");
    let id = s(t, "id");
    Ok(match kind.as_str() {
        "column" | "" => UiNode::Column { children: children(t, depth)?, spacing: num(t, "spacing").unwrap_or(8.0) as f32 },
        "row" => UiNode::Row { children: children(t, depth)?, spacing: num(t, "spacing").unwrap_or(8.0) as f32 },
        "card" => UiNode::Card { title: opt_s(t, "title"), children: children(t, depth)? },
        "scroll" => {
            let inner = UiNode::Column { children: children(t, depth)?, spacing: num(t, "spacing").unwrap_or(6.0) as f32 };
            UiNode::Scroll { child: Box::new(inner), height: num(t, "height").map(|h| h as f32) }
        }
        "text" => UiNode::Text { text: opt_s(t, "text").unwrap_or_else(|| s(t, "1")), size: num(t, "size").unwrap_or(13.0) as f32, color: color(t, "color"), bold: flag(t, "bold", false), mono: flag(t, "mono", false) },
        "heading" => UiNode::Heading { text: opt_s(t, "text").unwrap_or_else(|| s(t, "1")) },
        "separator" => UiNode::Separator,
        "space" => UiNode::Space { size: num(t, "size").unwrap_or(8.0) as f32 },
        "button" => UiNode::Button {
            label: opt_s(t, "label").unwrap_or_else(|| s(t, "1")),
            style: match s(t, "style").as_str() {
                "primary" => ButtonStyle::Primary,
                "danger" => ButtonStyle::Danger,
                "ghost" => ButtonStyle::Ghost,
                _ => ButtonStyle::Standard,
            },
            pick: match s(t, "pick").as_str() {
                "file" => Pick::File,
                "folder" => Pick::Folder,
                "save" => Pick::SaveFile,
                "world" => Pick::World,
                _ => Pick::None,
            },
            enabled: flag(t, "enabled", true),
            icon: opt_s(t, "icon"),
            id,
        },
        "input" => UiNode::Input { value: s(t, "value"), placeholder: s(t, "placeholder"), width: num(t, "width").map(|w| w as f32), id },
        "toggle" => UiNode::Toggle { label: s(t, "label"), value: flag(t, "value", false), id },
        "slider" => {
            let (min, max) = (num(t, "min").unwrap_or(0.0), num(t, "max").unwrap_or(100.0));
            UiNode::Slider { min, max, step: num(t, "step").unwrap_or(1.0), value: num(t, "value").unwrap_or(min).clamp(min.min(max), max.max(min)), label: s(t, "label"), id }
        }
        "select" => {
            let options = match t.get::<Value>("options")? {
                Value::Table(o) => o.sequence_values::<Value>().filter_map(Result::ok).map(|v| match v {
                    Value::String(x) => x.to_string_lossy(),
                    other => format!("{other:?}"),
                }).collect(),
                _ => Vec::new(),
            };
            UiNode::Select { options, value: opt_s(t, "value"), placeholder: s(t, "placeholder"), id }
        }
        "table" => {
            let columns: Vec<String> = match t.get::<Value>("columns")? {
                Value::Table(c) => c.sequence_values::<String>().filter_map(Result::ok).collect(),
                _ => Vec::new(),
            };
            let mut rows = Vec::new();
            if let Value::Table(r) = t.get::<Value>("rows")? {
                for row in r.sequence_values::<Table>() {
                    let row = row?;
                    rows.push(row.sequence_values::<Value>().filter_map(Result::ok).map(|v| match v {
                        Value::String(x) => x.to_string_lossy(),
                        Value::Integer(i) => i.to_string(),
                        Value::Number(n) => format!("{n}"),
                        Value::Boolean(b) => b.to_string(),
                        Value::Nil => String::new(),
                        other => format!("{other:?}"),
                    }).collect());
                }
            }
            // Lua rows are 1-based.
            let selected = num(t, "selected").filter(|v| *v >= 1.0).map(|v| v as usize - 1);
            UiNode::Table { columns, rows, selected, height: num(t, "height").map(|h| h as f32), id }
        }
        "chest" => {
            let mut items = Vec::new();
            if let Value::Table(list) = t.get::<Value>("items")? {
                for it in list.sequence_values::<Table>() {
                    items.push(parse_item(&it?));
                }
            }
            let cols = num(t, "cols").unwrap_or(9.0).clamp(1.0, 18.0) as usize;
            let min_rows = items.iter().map(|i| i.slot.max(0) as usize / cols + 1).max().unwrap_or(1);
            let rows = num(t, "rows").map_or(min_rows.max(3), |r| r.clamp(1.0, 12.0) as usize);
            UiNode::Chest { title: s(t, "title"), rows, cols, items, selected: num(t, "selected").map(|v| v as i32), id }
        }
        "block" => UiNode::Block { name: opt_s(t, "name").unwrap_or_else(|| s(t, "1")), size: num(t, "size").unwrap_or(32.0) as f32 },
        "progress" => UiNode::Progress { value: num(t, "value").unwrap_or(0.0).clamp(0.0, 1.0) as f32, label: s(t, "label") },
        "code" => UiNode::Code { text: opt_s(t, "text").unwrap_or_else(|| s(t, "1")) },
        "badge" => UiNode::Badge { text: opt_s(t, "text").unwrap_or_else(|| s(t, "1")), color: color(t, "color") },
        "image" => UiNode::Image { path: opt_s(t, "path").unwrap_or_else(|| s(t, "1")), size: num(t, "size").unwrap_or(64.0) as f32 },
        "number" => {
            let (min, max) = (num(t, "min").unwrap_or(f64::MIN), num(t, "max").unwrap_or(f64::MAX));
            UiNode::Number { value: num(t, "value").unwrap_or(0.0).clamp(min.min(max), max.max(min)), min, max, step: num(t, "step").unwrap_or(1.0), id }
        }
        other => return Err(mlua::Error::runtime(format!("unknown ui kind {other:?}"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prelude_builds_a_tree_the_host_parses() {
        let lua = Lua::new();
        lua.load(PRELUDE).exec().unwrap();
        let v: Value = lua
            .load(
                r##"return ui.column{
                    ui.heading("Chests"),
                    ui.row{ ui.input{ id = "x", value = 12 }, ui.button{ id = "go", label = "Scan", style = "primary" } },
                    "plain string",
                    nil,
                    ui.table{ id = "list", columns = {"Pos", "Items"}, rows = { {"1 2 3", 4}, {"5 6 7", 0} }, selected = 2 },
                    ui.chest{ id = "c", title = "Chest", items = { { slot = 0, id = "minecraft:diamond", count = 3, enchanted = true } } },
                    ui.text{ text = "red", color = "#ff0000" },
                    ui.badge{ text = "new", color = {0, 128, 255} },
                    ui.number{ id = "n", value = 5, min = 0, max = 10, step = 2 },
                    ui.image{ path = "/tmp/x.png", size = 48 },
                }"##,
            )
            .eval()
            .unwrap();
        let UiNode::Column { children, .. } = parse_value(&v, 0).unwrap().unwrap() else { panic!() };
        assert_eq!(children.len(), 9, "nil skipped, string becomes text");
        assert_eq!(children[0], UiNode::Heading { text: "Chests".into() });
        let UiNode::Row { children: r, .. } = &children[1] else { panic!() };
        assert!(matches!(&r[0], UiNode::Input { id, value, .. } if id == "x" && value == "12"));
        assert!(matches!(&r[1], UiNode::Button { style: ButtonStyle::Primary, .. }));
        assert!(matches!(&children[3], UiNode::Table { selected: Some(1), rows, .. } if rows[0] == ["1 2 3", "4"]));
        let UiNode::Chest { rows, cols, items, .. } = &children[4] else { panic!() };
        assert_eq!((*rows, *cols), (3, 9));
        assert!(items[0].glint && items[0].count == 3);
        assert!(matches!(&children[5], UiNode::Text { color: Some((255, 0, 0)), .. }));
        assert_eq!(children[6], UiNode::Badge { text: "new".into(), color: Some((0, 128, 255)) });
        assert!(matches!(&children[7], UiNode::Number { id, value, min, max, step } if id == "n" && *value == 5.0 && *min == 0.0 && *max == 10.0 && *step == 2.0));
        assert!(matches!(&children[8], UiNode::Image { path, size } if path == "/tmp/x.png" && *size == 48.0));
        assert!(parse_value(&lua.load("return { kind = 'nope' }").eval().unwrap(), 0).is_err());
    }

    #[test]
    fn events_become_lua_tables() {
        let lua = Lua::new();
        let t = ToolEvent::Row { id: "list".into(), row: 0 }.to_lua(&lua).unwrap();
        assert_eq!((t.get::<String>("kind").unwrap(), t.get::<i64>("row").unwrap()), ("row".into(), 1));
        let t = ToolEvent::MapClick { x: 5, y: None, z: -3, dim: 0 }.to_lua(&lua).unwrap();
        assert_eq!(t.get::<String>("kind").unwrap(), "map_click");
    }
}
