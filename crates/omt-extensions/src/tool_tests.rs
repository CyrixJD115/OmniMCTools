//! End-to-end: a temp extension with a tool + an NBT action, run through
//! the worker host against a mock world holding a Bedrock chest.

use std::path::Path;
use std::sync::{Arc, Mutex};

use omt_nbt::Tag;
use omt_world::{Chunk, Edition, WorldNode, WorldResult, WorldStore};

use super::*;
use crate::runtime::WorldHandle;

/// Chunks + the last block a fill asked for.
struct Mock(Mutex<Vec<Chunk>>, Mutex<Option<String>>);

impl WorldStore for Mock {
    fn read_chunk(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<Chunk>> {
        Ok(self.0.lock().unwrap().iter().find(|c| (c.dim, c.x, c.z) == (dim, x, z)).cloned())
    }
    fn write_chunk(&self, dim: u8, x: i32, z: i32, chunk: &Chunk) -> WorldResult<()> {
        let mut g = self.0.lock().unwrap();
        g.retain(|c| (c.dim, c.x, c.z) != (dim, x, z));
        g.push(chunk.clone());
        Ok(())
    }
    fn delete_chunk(&self, _: u8, _: i32, _: i32) -> WorldResult<()> {
        Ok(())
    }
    fn iter_chunks(&self, dim: u8) -> WorldResult<Box<dyn Iterator<Item = (i32, i32)> + '_>> {
        let v: Vec<(i32, i32)> = self.0.lock().unwrap().iter().filter(|c| c.dim == dim).map(|c| (c.x, c.z)).collect();
        Ok(Box::new(v.into_iter()))
    }
    fn read_raw(&self, node: &WorldNode) -> WorldResult<Vec<u8>> {
        Err(omt_world::WorldError::NoSuchNode(node.clone()))
    }
    fn flush(&mut self) -> WorldResult<()> {
        Ok(())
    }
}

/// Writes block entities / blocks into the mock's chunks.
struct MockWriter(Arc<Mock>);

impl crate::runtime::WorldWriter for MockWriter {
    fn set_block_entities(&self, dim: u8, cx: i32, cz: i32, list: &[Tag]) -> Result<(), String> {
        let mut g = self.0.0.lock().unwrap();
        let c = g.iter_mut().find(|c| (c.dim, c.x, c.z) == (dim, cx, cz)).ok_or("no chunk")?;
        c.block_entities = list.to_vec();
        Ok(())
    }
    fn fill(&self, _: u8, a: [i32; 3], b: [i32; 3], block: &str, _: Option<&str>) -> Result<u64, String> {
        *self.0.1.lock().unwrap() = Some(block.to_string());
        Ok(((b[0] - a[0]).abs() + 1) as u64 * ((b[1] - a[1]).abs() + 1) as u64 * ((b[2] - a[2]).abs() + 1) as u64)
    }
}

fn c(e: Vec<(&str, Tag)>) -> Tag {
    Tag::Compound(e.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn chest() -> Tag {
    let item = |slot: i8, name: &str, n: i8| c(vec![("Count", Tag::Byte(n)), ("Name", Tag::String(name.into())), ("Slot", Tag::Byte(slot))]);
    c(vec![
        ("id", Tag::String("Chest".into())),
        ("x", Tag::Int(20)),
        ("y", Tag::Int(64)),
        ("z", Tag::Int(-5)),
        ("Items", Tag::List(vec![item(0, "minecraft:diamond", 3), item(13, "minecraft:oak_log", 64)])),
    ])
}

const MAIN: &str = r#"
local Tool = { type = EXTENSION_TYPE.TOOL, id = "chests", name = "Chests", icon = "box", map = true }
function Tool:init(ctx)
  self.found = ctx.world:containers(0, 0, -16, 31, 15)
  self.clicks = 0
  omt.storage.set("opened", (omt.storage.get("opened") or 0) + 1)
end
function Tool:on_event(ev, ctx)
  if ev.kind == "click" and ev.id == "more" then
    self.clicks = self.clicks + 1
    local t = self.found[1].tag
    local items = t:getList("Items")
    items:addChild(omt.items.make{ id = "gold_ingot", count = 5, slot = 4, edition = "BEDROCK" })
    ctx.world:set_block_entity(0, self.found[1].x, self.found[1].y, self.found[1].z, t)
    self.found = ctx.world:containers(0, 0, -16, 31, 15)
    omt.notify("added gold")
  elseif ev.kind == "click" and ev.id == "boom" then
    error("kaboom")
  end
end
function Tool:view(ctx)
  local c = self.found[1]
  return ui.column{
    ui.heading(#self.found .. " container(s)"),
    ui.chest{ id = "chest", title = c.kind, rows = c.rows, cols = c.cols, items = c.items },
    ui.text(omt.json.encode({ clicks = self.clicks })),
  }
end

local Act = { type = EXTENSION_TYPE.NBT_ACTION, id = "double", name = "Double counts", names = { "Items" }, types = { TYPE.LIST } }
function Act:init(ctx)
  for i = 0, ctx.node.childCount - 1 do
    local it = ctx.node:child(i)
    it:setByte("Count", math.min(it:getByte("Count") * 2, 64))
  end
end
function Act:view(ctx)
  return ui.chest{ id = "c", items = omt.items.read(ctx.node) }
end
return { Tool, Act }
"#;

fn setup(tag: &str) -> (std::path::PathBuf, WorldHandle, Arc<Mock>) {
    let root = std::env::temp_dir().join(format!("omt-ext-tool-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("demo")).unwrap();
    std::fs::write(root.join("demo/main.lua"), MAIN).unwrap();
    let chunk = Chunk { x: 1, z: -1, dim: 0, block_entities: vec![chest()], ..Default::default() };
    let mock = Arc::new(Mock(Mutex::new(vec![chunk]), Mutex::new(None)));
    let writer: Arc<dyn crate::runtime::WorldWriter> = Arc::new(MockWriter(mock.clone()));
    let h = WorldHandle { store: mock.clone(), edition: Edition::Bedrock, path: root.clone(), name: "Mock".into(), writable: true, writer: Some(writer) };
    (root, h, mock)
}

#[test]
fn discovers_tools_and_actions() {
    let (root, _, _) = setup("disc");
    let (decls, errs) = discover(&root, &["demo".to_string()]);
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(decls.len(), 2);
    let tool = decls.iter().find(|d| d.kind == ToolKind::Tool).unwrap();
    assert_eq!((tool.key().as_str(), tool.name.as_str(), tool.icon.as_str(), tool.map), ("demo/chests", "Chests", "box", true));
    let act = decls.iter().find(|d| d.kind == ToolKind::NbtAction).unwrap();
    assert!(act.applies("Items", 9) && !act.applies("Items", 10) && !act.applies("Inventory", 9));
    let (_, errs) = discover(&root, &["missing".to_string()]);
    assert_eq!(errs.len(), 1, "a broken extension is reported, not fatal");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tool_reads_world_shows_chest_and_writes_back() {
    let (root, h, mock) = setup("run");
    let (host, first) = ToolHost::start(ToolSpec { root: root.clone(), ext: "demo".into(), tool: "chests".into(), world: Some(h), ..Default::default() });
    let out = first.recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(out.title, "Chests");
    let Some(UiNode::Column { children, .. }) = out.view else { panic!("{:?}", out.view) };
    assert_eq!(children[0], UiNode::Heading { text: "1 container(s)".into() });
    let UiNode::Chest { rows, cols, items, title, .. } = &children[1] else { panic!() };
    assert_eq!((*rows, *cols, title.as_str()), (3, 9, "chest"));
    assert_eq!(items.iter().map(|i| (i.slot, i.id.as_str(), i.count)).collect::<Vec<_>>(), [(0, "minecraft:diamond", 3), (13, "minecraft:oak_log", 64)]);

    // A click edits the chest in the world (block entity write-back).
    let out = host.send(ToolEvent::Click { id: "more".into() }).recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert!(out.effects.dirty && out.effects.notes == ["added gold"]);
    let Some(UiNode::Column { children, .. }) = out.view else { panic!() };
    let UiNode::Chest { items, .. } = &children[1] else { panic!() };
    assert!(items.iter().any(|i| i.id == "minecraft:gold_ingot" && i.count == 5 && i.slot == 4));
    let stored = mock.read_chunk(0, 1, -1).unwrap().unwrap();
    let crate::items::Item { count, .. } = crate::items::items(match &stored.block_entities[0] {
        Tag::Compound(e) => &e.iter().find(|(k, _)| k == "Items").unwrap().1,
        _ => panic!(),
    })
    .into_iter()
    .find(|i| i.id == "minecraft:gold_ingot")
    .unwrap();
    assert_eq!(count, 5, "written to the store");

    // Errors are reported and the tool keeps working.
    let out = host.send(ToolEvent::Click { id: "boom".into() }).recv().unwrap();
    assert!(out.error.as_deref().is_some_and(|e| e.contains("kaboom")));
    assert!(out.view.is_some());
    assert!(std::fs::read_to_string(root.join("demo/storage.json")).unwrap().contains("\"opened\": 1"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn nbt_action_edits_the_selected_tag() {
    let (root, _, _) = setup("act");
    let Tag::Compound(e) = chest() else { panic!() };
    let items = e.into_iter().find(|(k, _)| k == "Items").unwrap().1;
    let (_host, first) = ToolHost::start(ToolSpec { root: root.clone(), ext: "demo".into(), tool: "double".into(), node: Some(("Items".into(), items)), ..Default::default() });
    let out = first.recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let replaced = out.effects.replace.expect("edited node handed back");
    let counts: Vec<i32> = crate::items::items(&replaced).iter().map(|i| i.count).collect();
    assert_eq!(counts, [6, 64], "doubled, capped at a stack");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stop_cancels_a_runaway_callback() {
    let root = std::env::temp_dir().join(format!("omt-ext-tool-loop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("loop")).unwrap();
    std::fs::write(
        root.join("loop/main.lua"),
        r#"return { { type = EXTENSION_TYPE.TOOL, id = "spin",
             on_event = function(self, ev) while true do end end,
             view = function(self) return ui.text("ok") end } }"#,
    )
    .unwrap();
    let (host, first) = ToolHost::start(ToolSpec { root: root.clone(), ext: "loop".into(), tool: "spin".into(), ..Default::default() });
    assert!(first.recv().unwrap().error.is_none());
    let rx = host.send(ToolEvent::Click { id: "x".into() });
    std::thread::sleep(std::time::Duration::from_millis(100));
    host.cancel();
    let out = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("stopped");
    assert!(out.error.as_deref().is_some_and(|e| e.contains("cancelled")), "{:?}", out.error);
    let _ = Path::new(&root);
    let _ = std::fs::remove_dir_all(&root);
}

/// The shipped example runs against the mock world: scan, search, select,
/// open a slot, edit (writable) — and the NBT action sorts.
#[test]
fn shipped_example_works() {
    let (root, h, mock) = setup("example");
    std::fs::create_dir_all(root.join("chest-explorer")).unwrap();
    for (rel, bytes) in crate::example_files() {
        std::fs::write(root.join("chest-explorer").join(rel), bytes).unwrap();
    }
    let (decls, errs) = discover(&root, &["chest-explorer".to_string()]);
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(decls.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["chest-explorer", "open-as-chest", "players"]);
    let spec = ToolSpec { root: root.clone(), ext: "chest-explorer".into(), tool: "chest-explorer".into(), world: Some(h), ..Default::default() };
    let (host, first) = ToolHost::start(spec);
    let out = first.recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let chest_of = |v: &Option<UiNode>| -> Option<Vec<(i32, String, i32)>> {
        fn find(n: &UiNode) -> Option<&UiNode> {
            match n {
                UiNode::Chest { .. } => Some(n),
                UiNode::Column { children, .. } | UiNode::Row { children, .. } | UiNode::Card { children, .. } => children.iter().find_map(find),
                _ => None,
            }
        }
        match find(v.as_ref()?)? {
            UiNode::Chest { items, .. } => Some(items.iter().map(|i| (i.slot, i.id.clone(), i.count)).collect()),
            _ => None,
        }
    };
    // Spawn is 0,0 (no level.dat): the chest at 20,-5 is inside 64 blocks.
    assert_eq!(chest_of(&out.view).unwrap(), [(0, "minecraft:diamond".to_string(), 3), (13, "minecraft:oak_log".to_string(), 64)]);
    // Search for something absent → no chest; then diamonds → found.
    let out = host.send(ToolEvent::Submit { id: "search".into(), value: "netherite".into() }).recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert!(chest_of(&out.view).is_none());
    let out = host.send(ToolEvent::Submit { id: "search".into(), value: "diamond".into() }).recv().unwrap();
    assert_eq!(out.effects.notes, ["3 × diamond in 1 container(s)"]);
    // Open slot 0, then fill the stack (writable world) → written back.
    let out = host.send(ToolEvent::Slot { id: "chest".into(), slot: 0 }).recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let out = host.send(ToolEvent::Click { id: "fill".into() }).recv().unwrap();
    assert!(out.error.is_none() && out.effects.dirty, "{:?}", out.error);
    assert_eq!(chest_of(&out.view).unwrap()[0].2, 64);
    let stored = mock.read_chunk(0, 1, -1).unwrap().unwrap();
    assert!(omt_nbt::snbt::to_snbt(&stored.block_entities[0]).contains("Count:64b"));
    // A map click re-centres far away → nothing there.
    let out = host.send(ToolEvent::MapClick { x: 5000, y: None, z: 5000, dim: 0 }).recv().unwrap();
    assert!(out.error.is_none() && chest_of(&out.view).is_none(), "{:?}", out.error);

    // NBT action: sort by id renumbers slots and hands the list back.
    let Tag::Compound(e) = chest() else { panic!() };
    let items = e.into_iter().find(|(k, _)| k == "Items").unwrap().1;
    let (act, first) = ToolHost::start(ToolSpec { root: root.clone(), ext: "chest-explorer".into(), tool: "open-as-chest".into(), node: Some(("Items".into(), items)), ..Default::default() });
    assert!(first.recv().unwrap().error.is_none());
    let out = act.send(ToolEvent::Click { id: "sort".into() }).recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let sorted = crate::items::items(&out.effects.replace.expect("edited"));
    assert_eq!(sorted.iter().map(|i| (i.slot, i.id.as_str())).collect::<Vec<_>>(), [(0, "minecraft:diamond"), (1, "minecraft:oak_log")]);
    let _ = std::fs::remove_dir_all(&root);
}

/// `w:set_block` / `w:fill` go through the app's writer; read-only worlds
/// refuse every write with a clear message.
#[test]
fn block_writes_use_the_writer_and_respect_read_only() {
    let (root, h, mock) = setup("blocks");
    std::fs::create_dir_all(root.join("blk")).unwrap();
    std::fs::write(
        root.join("blk/main.lua"),
        r#"return { { type = EXTENSION_TYPE.TOOL, id = "b",
             init = function(self, ctx)
               self.one = ctx.world:set_block(0, 1, 64, 2, "oak_log[axis=y]")
               self.n = ctx.world:fill(0, 0, 60, 0, 3, 61, 3, "glass", { replace = "dirt" })
               self.big = pcall(function() ctx.world:fill(0, 0, 0, 0, 1000, 300, 1000, "air") end)
             end,
             view = function(self) return ui.text(tostring(self.one) .. " " .. self.n .. " " .. tostring(self.big)) end } }"#,
    )
    .unwrap();
    let (_host, first) = ToolHost::start(ToolSpec { root: root.clone(), ext: "blk".into(), tool: "b".into(), world: Some(h.clone()), ..Default::default() });
    let out = first.recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(out.view, Some(UiNode::Text { text: "true 32 false".into(), size: 13.0, color: None, bold: false, mono: false }));
    assert!(out.effects.dirty);
    assert_eq!(mock.1.lock().unwrap().as_deref(), Some("glass"));

    let ro = WorldHandle { writable: false, writer: None, ..h };
    let (_host, first) = ToolHost::start(ToolSpec { root: root.clone(), ext: "blk".into(), tool: "b".into(), world: Some(ro), ..Default::default() });
    let out = first.recv().unwrap();
    assert!(out.error.as_deref().is_some_and(|e| e.contains("read-only")), "{:?}", out.error);
    let _ = std::fs::remove_dir_all(&root);
}

/// The "New extension" template is a working tool out of the box.
#[test]
fn template_runs() {
    let (root, h, _) = setup("template");
    std::fs::create_dir_all(root.join("mine")).unwrap();
    for (rel, bytes) in crate::template_files("My Tool") {
        std::fs::write(root.join("mine").join(rel), bytes).unwrap();
    }
    let (decls, errs) = discover(&root, &["mine".to_string()]);
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!((decls[0].name.as_str(), decls[0].map), ("My Tool", true));
    let (host, first) = ToolHost::start(ToolSpec { root: root.clone(), ext: "mine".into(), tool: "main".into(), world: Some(h), ..Default::default() });
    let out = first.recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let out = host.send(ToolEvent::Click { id: "hello".into() }).recv().unwrap();
    assert_eq!(out.effects.notes, ["hello #1"]);
    let out = host.send(ToolEvent::MapClick { x: 20, y: None, z: -5, dim: 0 }).recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert!(format!("{:?}", out.view).contains("Map click: 20, -5"));
    let _ = std::fs::remove_dir_all(&root);
}

/// `omt.items.put` works on a container without `Items` (empty Java chest)
/// and replaces a stack already in the slot.
#[test]
fn items_put_creates_and_replaces() {
    let (root, _, _) = setup("put");
    std::fs::create_dir_all(root.join("put")).unwrap();
    std::fs::write(
        root.join("put/main.lua"),
        r#"return { { type = EXTENSION_TYPE.TOOL, id = "p",
             init = function(self)
               local chest = omt.nbt.new{ id = "minecraft:chest", x = 1, y = 2, z = 3 }
               omt.items.put(chest, omt.items.make{ id = "diamond", count = 5, slot = 4, edition = "JAVA" })
               omt.items.put(chest, omt.items.make{ id = "emerald", count = 2, slot = 4, edition = "JAVA" })
               omt.items.put(chest, omt.items.make{ id = "stone", count = 1, slot = 0, edition = "JAVA" })
               local list = omt.items.read(chest)
               self.out = #list .. " " .. list[1].id .. " " .. list[2].id .. " " .. list[2].count
             end,
             view = function(self) return ui.text(self.out) end } }"#,
    )
    .unwrap();
    let (_h, first) = ToolHost::start(ToolSpec { root: root.clone(), ext: "put".into(), tool: "p".into(), ..Default::default() });
    let out = first.recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(out.view, Some(UiNode::Text { text: "2 minecraft:emerald minecraft:stone 1".into(), size: 13.0, color: None, bold: false, mono: false }));
    let _ = std::fs::remove_dir_all(&root);
}
