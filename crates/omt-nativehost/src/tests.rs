//! End-to-end: build the sample plugin's `cdylib`, load it, and drive it
//! against a mock world — discover its entries, open the Containers tool,
//! read a chest, fill a slot (world write), and run the Fill-stacks action.

use std::path::PathBuf;
use std::sync::{Mutex, Once};

use omt_extensions::runtime::{WorldEntry, WorldHandle, WorldWriter};
use omt_extensions::tool::{ToolKind, ToolSpec};
use omt_extensions::ui::{ToolEvent, UiNode};
use omt_nbt::Tag;
use omt_world::{Chunk, Edition, WorldNode, WorldResult, WorldStore};

use super::*;

// --- mock world -----------------------------------------------------------

struct Mock(Mutex<Vec<Chunk>>);

impl WorldStore for Mock {
    fn read_chunk(&self, dim: u8, x: i32, z: i32) -> WorldResult<Option<Chunk>> {
        Ok(self.0.lock().unwrap().iter().find(|c| (c.dim, c.x, c.z) == (dim, x, z)).cloned())
    }
    fn write_chunk(&self, _: u8, _: i32, _: i32, _: &Chunk) -> WorldResult<()> {
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

struct MockWriter(Arc<Mock>);

impl WorldWriter for MockWriter {
    fn set_block_entities(&self, dim: u8, cx: i32, cz: i32, list: &[Tag]) -> Result<(), String> {
        let mut g = self.0.0.lock().unwrap();
        let c = g.iter_mut().find(|c| (c.dim, c.x, c.z) == (dim, cx, cz)).ok_or("no chunk")?;
        c.block_entities = list.to_vec();
        Ok(())
    }
    fn fill(&self, _: u8, _: [i32; 3], _: [i32; 3], _: &str, _: Option<&str>) -> Result<u64, String> {
        Ok(1)
    }
}

fn c(e: Vec<(&str, Tag)>) -> Tag {
    Tag::Compound(e.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn chest() -> Tag {
    let item = |slot: i8, name: &str, n: i8| c(vec![("Count", Tag::Byte(n)), ("Name", Tag::String(name.into())), ("Slot", Tag::Byte(slot))]);
    c(vec![
        ("id", Tag::String("Chest".into())),
        ("x", Tag::Int(4)),
        ("y", Tag::Int(64)),
        ("z", Tag::Int(4)),
        ("Items", Tag::List(vec![item(0, "minecraft:diamond", 3), item(1, "minecraft:oak_log", 12)])),
    ])
}

// --- build the sample cdylib once -----------------------------------------

fn workspace_root() -> PathBuf {
    // crates/omt-nativehost → workspace root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

fn dylib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "omt_native_sample.dll"
    } else if cfg!(target_os = "macos") {
        "libomt_native_sample.dylib"
    } else {
        "libomt_native_sample.so"
    }
}

static BUILD: Once = Once::new();

/// Build the sample and return the path to its dynamic library.
fn sample_lib() -> PathBuf {
    let root = workspace_root();
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("target"));
    let find = || ["debug", "release"].iter().map(|p| target.join(p).join(dylib_name())).find(|p| p.is_file());
    if let Some(p) = find() {
        return p;
    }
    BUILD.call_once(|| {
        let _ = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())).args(["build", "-p", "omt-native-sample"]).current_dir(&root).status();
    });
    find().expect("sample cdylib should be built")
}

/// Install the sample under a temp extensions root as folder `native`.
fn setup(tag: &str) -> (PathBuf, WorldHandle, Arc<Mock>) {
    let lib = sample_lib();
    let root = std::env::temp_dir().join(format!("omt-native-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("native")).unwrap();
    std::fs::copy(&lib, root.join("native").join(lib.file_name().unwrap())).unwrap();
    let chunk = Chunk { x: 0, z: 0, dim: 0, block_entities: vec![chest()], ..Default::default() };
    let mock = Arc::new(Mock(Mutex::new(vec![chunk])));
    let writer: Arc<dyn WorldWriter> = Arc::new(MockWriter(mock.clone()));
    let h = WorldHandle { store: mock.clone(), edition: Edition::Bedrock, path: root.clone(), name: "Mock".into(), writable: true, writer: Some(writer) };
    (root, h, mock)
}

fn spec(root: &std::path::Path, tool: &str, world: Option<WorldHandle>, node: Option<(String, Tag)>) -> ToolSpec {
    ToolSpec { root: root.to_path_buf(), ext: "native".into(), tool: tool.into(), world, opener: None, library: vec![WorldEntry { name: "Mock".into(), path: root.to_path_buf(), edition: Edition::Bedrock }], node }
}

fn chest_items(v: &Option<UiNode>) -> Option<Vec<(i32, String, i32)>> {
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
}

#[test]
fn native_plugin_discovers_runs_reads_and_writes() {
    let (root, h, mock) = setup("run");
    let (decls, errs) = discover_native(&root, &["native".to_string()]);
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(decls.iter().map(|d| (d.key(), d.kind, d.map)).collect::<Vec<_>>(), vec![("native/containers".to_string(), ToolKind::Tool, true), ("native/fill-stacks".to_string(), ToolKind::NbtAction, false)]);
    assert!(decls[1].applies("Items", 9) && !decls[1].applies("Items", 10));

    // Open the Containers tool; map-click (0,0) scans the mock chest.
    let (host, first) = NativeHost::start(spec(&root, "containers", Some(h.clone()), None));
    let out = first.recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(out.title, "Containers");
    let out = host.send(ToolEvent::MapClick { x: 0, y: None, z: 0, dim: 0 }).recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(out.effects.notes, ["1 container(s) near 0, 0"]);
    assert_eq!(chest_items(&out.view).unwrap(), [(0, "minecraft:diamond".to_string(), 3), (1, "minecraft:oak_log".to_string(), 12)]);

    // Select slot 1 and fill it → the mock store is written.
    let out = host.send(ToolEvent::Slot { id: "chest".into(), slot: 1 }).recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let out = host.send(ToolEvent::Click { id: "fill".into() }).recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    assert!(out.effects.dirty, "world marked modified");
    assert_eq!(chest_items(&out.view).unwrap().iter().find(|(s, ..)| *s == 1).unwrap().2, 64);
    let stored = mock.read_chunk(0, 0, 0).unwrap().unwrap();
    assert!(omt_nbt::snbt::to_snbt(&stored.block_entities[0]).contains("Count:64b"));
    Box::new(host).shutdown();

    // The NBT action edits the selected Items list, handed back as a tag.
    let Tag::Compound(e) = chest() else { panic!() };
    let items = e.into_iter().find(|(k, _)| k == "Items").unwrap().1;
    let (act, first) = NativeHost::start(spec(&root, "fill-stacks", None, Some(("Items".into(), items))));
    assert!(first.recv().unwrap().error.is_none());
    let out = act.send(ToolEvent::Click { id: "fill".into() }).recv().unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let replaced = out.effects.replace.expect("edited node returned");
    let counts: Vec<i64> = match replaced {
        Tag::List(l) => l.iter().filter_map(|t| match t {
            Tag::Compound(e) => e.iter().find(|(k, _)| k == "Count").and_then(|(_, v)| if let Tag::Byte(b) = v { Some(i64::from(*b)) } else { None }),
            _ => None,
        }).collect(),
        _ => panic!("expected a list"),
    };
    assert_eq!(counts, [64, 64]);
    let _ = std::fs::remove_dir_all(&root);
}

/// A panic inside the plugin is caught and reported, not an abort.
#[test]
fn errors_do_not_crash_the_host() {
    let (root, h, _) = setup("err");
    let (host, first) = NativeHost::start(spec(&root, "containers", Some(h), None));
    assert!(first.recv().unwrap().error.is_none());
    // An unknown event id is simply ignored; the tool keeps rendering.
    let out = host.send(ToolEvent::Click { id: "nope".into() }).recv().unwrap();
    assert!(out.error.is_none() && out.view.is_some());
    let _ = std::fs::remove_dir_all(&root);
}
