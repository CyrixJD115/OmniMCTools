//! End-to-end tests: real extension files, real zip install, real walks.
//! These verify the *protocol* (registration, main-then-walk order, depth-first
//! recursion, styling collection, edit write-back) against Lua code that uses
//! the API the way a community extension would.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use omt_nbt::{Tag, TagPath};
use omt_script::{file_type, example_files, Context, Edition, Manager, RowStyle};

fn ctx() -> Context {
    Context { file_type: file_type::LEVEL, edition: Edition::Bedrock }
}

fn tmp_root(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("omt-script-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

fn install_src(mgr: &Manager, id: &str, main_lua: &str) {
    let files = vec![
        ("manifest.json".to_string(), format!(r#"{{ "name": "{id}", "version": "1.0" }}"#).into_bytes()),
        ("main.lua".to_string(), main_lua.as_bytes().to_vec()),
    ];
    mgr.install_files(id, &files, None).unwrap();
}

fn level_dat() -> Tag {
    Tag::Compound(vec![
        ("Time".into(), Tag::Long(6000)),
        ("DayTime".into(), Tag::Long(6000)),
        ("InhabitedTime".into(), Tag::Long(72_000)),
        ("spawnPoint".into(), Tag::Compound(vec![("x".into(), Tag::Int(3))])),
        ("names".into(), Tag::List(vec![Tag::String("a".into()), Tag::String("b".into())])),
    ])
}

/// Path helper: `.key` / `.key[0]` display strings, matching editor rows.
fn p(segs: &[&str]) -> String {
    let mut path = TagPath::root();
    for s in segs {
        if let Some(idx) = s.strip_prefix('[') {
            path = path.push_index(idx.trim_end_matches(']').parse().unwrap());
        } else {
            path = path.push_key(*s);
        }
    }
    path.to_string()
}

#[test]
fn example_extension_styles_tick_rows() {
    let root = tmp_root("example");
    let mgr = Manager::open(&root);
    mgr.install_files("ticks-time", &example_files(), None).unwrap();

    let run = mgr.run("", &level_dat(), &ctx());
    assert!(run.errors.is_empty(), "errors: {errors:?}", errors = run.errors);

    // Time 6000 = 6am + 6000 ticks = 12:00, day 0.
    assert_eq!(run.styles.get(&p(&["Time"])).unwrap().extra_text.as_deref(), Some("day 0, 12:00 in-game"));
    // InhabitedTime 72000 ticks = 3600 s = 1h.
    assert_eq!(run.styles.get(&p(&["InhabitedTime"])).unwrap().extra_text.as_deref(), Some("1h 00m 00s played"));
    let time_style = run.styles.get(&p(&["Time"])).unwrap();
    assert_eq!(time_style.color, Some((0x7f, 0xb0, 0x69)));
    assert!(time_style.description.as_deref().unwrap().starts_with("Tick Clock: time"));
    // Non-tick rows untouched.
    assert!(!run.styles.contains_key(&p(&["spawnPoint", "x"])));
    assert!(run.edited_root.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn visit_order_is_depth_first_and_main_runs_first() {
    let root = tmp_root("order");
    let mgr = Manager::open(&root);
    let main_lua = r#"
local Log = {}
Log.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
Log.recursive = true
function Log:main(root_tag, context)
  logger:info("main(root=" .. root_tag.type .. ",ctx=" .. context.type .. ")")
end
function Log:recursion(root_tag, current, context)
  local label = current.name
  if label == "" then label = "[" .. current:getRow() .. "]" end
  logger:info(label)
  if current.name == "spawnPoint" then
    logger:info("childCount=" .. current.childCount
      .. " viaChild=" .. current:child(0).name
      .. " parentOk=" .. tostring(current:child(0).parentTag.name == "spawnPoint"))
  end
  if current.type == TYPE.LIST then
    logger:info("listType=" .. current.listType .. " row1=" .. current:child(1):getRow())
  end
end
return { Log }
"#;
    install_src(&mgr, "order", main_lua);

    let run = mgr.run("", &level_dat(), &ctx());
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    let events: Vec<&str> = run.logs.iter().map(|(_, m)| m.as_str()).collect();
    // `main` runs before the walk; the walk is depth-first preorder: compound
    // children (and their subtrees) before the next sibling, list items in
    // order. Root first (empty name, row -1).
    assert_eq!(
        events,
        vec![
            "main(root=10,ctx=8)",
            "[-1]",
            "Time",
            "DayTime",
            "InhabitedTime",
            "spawnPoint",
            "childCount=1 viaChild=x parentOk=true",
            "x",
            "names",
            "listType=8 row1=1",
            "[0]",
            "[1]",
        ]
    );
    assert_eq!(run.visited, 9);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn edits_write_back_through_materialize() {
    let root = tmp_root("edit");
    let mgr = Manager::open(&root);
    let main_lua = r#"
local Fix = {}
Fix.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
Fix.recursive = true
function Fix:recursion(root_tag, current, context)
  if current.name == "Time" and current.type == TYPE.LONG then
    current:setLong(42)
  end
  if current.name == "names" and current.type == TYPE.LIST then
    current:addString("c")
  end
  if current.name == "spawnPoint" and current.type == TYPE.COMPOUND then
    current:addInt("z", -5)
    current:removeChild("x")
  end
end
return { Fix }
"#;
    install_src(&mgr, "edit", main_lua);

    let run = mgr.run("", &level_dat(), &ctx());
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    let edited = run.edited_root.expect("tree changed");
    let Tag::Compound(entries) = &edited else { panic!("root kind") };
    let get = |k: &str| entries.iter().find(|(ek, _)| ek == k).map(|(_, v)| v);
    assert_eq!(get("Time"), Some(&Tag::Long(42)));
    assert_eq!(get("names"), Some(&Tag::List(vec![Tag::String("a".into()), Tag::String("b".into()), Tag::String("c".into())])));
    match get("spawnPoint") {
        Some(Tag::Compound(entries)) => {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0], ("z".to_string(), Tag::Int(-5)));
        }
        other => panic!("spawnPoint: {other:?}"),
    }
    // An unmodified run reports no edit.
    let main_lua_noop = "local N = {}\nN.type = EXTENSION_TYPE.NBT_EDITOR_STYLE\nN.recursive = true\nreturn { N }";
    install_src(&mgr, "noop", main_lua_noop);
    mgr.set_enabled("edit", false).unwrap();
    let run2 = mgr.run("", &level_dat(), &ctx());
    assert!(run2.edited_root.is_none());
    assert!(run2.errors.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn typed_api_and_errors() {
    let root = tmp_root("api");
    let mgr = Manager::open(&root);
    let main_lua = r#"
local Probe = {}
Probe.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
Probe.recursive = true
function Probe:recursion(root_tag, current, context)
  if current.name == "spawnPoint" then
    current:setExtraText("getInt=" .. tostring(current:getInt("x"))
      .. " getMissing=" .. tostring(current:getInt("nope"))
      .. " getStringWrongType=" .. tostring(current:getString("x"))
      .. " contains=" .. tostring(current:contains("x"))
      .. " find=" .. tostring(current:find("x") ~= nil)
      .. " hasParent=" .. tostring(current:hasParent())
      .. " cloneType=" .. current:clone().type
      .. " value=" .. tostring(current.value)
      .. " childOOR=" .. tostring(current:child(99)))
  end
  if current.name == "Time" then
    current:setExtraText("setValueKeepsType")
    current:setValue(7)
    current:setDouble(1.5) -- must fail: Time is a Long
  end
end
return { Probe }
"#;
    install_src(&mgr, "probe", main_lua);
    let run = mgr.run("", &level_dat(), &ctx());
    // setDouble on a Long is a per-callback error; the walk continues and
    // other styles were applied.
    assert!(run.errors.len() == 1, "errors: {:?}", run.errors);
    assert!(run.errors[0].contains("setDouble"), "{:?}", run.errors[0]);
    let style = run.styles.get(&p(&["spawnPoint"])).unwrap();
    let text = style.extra_text.as_deref().unwrap();
    assert_eq!(
        text,
        "getInt=3 getMissing=nil getStringWrongType=nil contains=true find=true \
         hasParent=true cloneType=10 value=nil childOOR=nil"
    );
    // setValue(7) preserved the Long kind and 7 landed.
    assert_eq!(run.edited_root.as_ref().map(|t| t.get("Time").cloned()), Some(Some(Tag::Long(7))));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn broken_extension_is_reported_not_fatal() {
    let root = tmp_root("broken");
    let mgr = Manager::open(&root);
    install_src(&mgr, "good", r#"
local G = {}
G.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
G.recursive = true
function G:recursion(root_tag, current, context)
  if current.name == "Time" then current:setExtraText("good") end
end
return { G }
"#);
    mgr.install_files(
        "bad",
        &[
            ("manifest.json".into(), br#"{ "name": "bad" }"#.to_vec()),
            ("main.lua".into(), b"return {}\nerror('boom')".to_vec()),
        ],
        None,
    )
    .unwrap();
    install_src(&mgr, "boom", "local B = {}\nB.type = EXTENSION_TYPE.NBT_EDITOR_STYLE\nB.recursive = true\nfunction B:recursion() error('kaboom') end\nreturn { B }");

    let run = mgr.run("", &level_dat(), &ctx());
    assert!(run.errors.iter().any(|e| e.contains("boom")));
    assert!(run.errors.iter().any(|e| e.contains("kaboom")));
    assert_eq!(run.styles.get(&p(&["Time"])).unwrap().extra_text.as_deref(), Some("good"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn logger_and_print_are_captured() {
    let root = tmp_root("log");
    let mgr = Manager::open(&root);
    install_src(&mgr, "log", r#"
local L = {}
L.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
L.recursive = true
function L:main(root_tag, context)
  logger:info("hello from " .. BASE_PATH)
  print("printed", 42)
end
return { L }
"#);
    let run = mgr.run("", &level_dat(), &ctx());
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert!(run.logs.iter().any(|(l, m)| l == "info" && m.contains("hello from")));
    assert!(run.logs.iter().any(|(l, m)| l == "print" && m == "printed 42"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn zip_run_and_context_types() {
    let root = tmp_root("ziprun");
    let mgr = Manager::open(&root);
    mgr.install_files("ticks-time", &example_files(), None).unwrap();

    let file = std::fs::File::create(root.join("repack.zip")).unwrap();
    let mut z = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    z.start_file("manifest.json", opts).unwrap();
    z.write_all(br#"{ "name": "Ctx Probe", "version": "1" }"#).unwrap();
    z.start_file("main.lua", opts).unwrap();
    z.write_all(
        br#"
local C = {}
C.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
C.recursive = true
function C:recursion(root_tag, current, context)
  if current.name == "Time" then
    current:setExtraText(
      (context.type & FILE_TYPE.LEVEL) ~= 0 and "level" or "other")
    current:setDescription("edition=" .. context.edition)
  end
end
return { C }
"#,
    )
    .unwrap();
    z.finish().unwrap();
    let id = mgr.install_zip(&root.join("repack.zip")).unwrap();
    assert_eq!(id, "ctx-probe");

    let run = mgr.run("", &level_dat(), &Context { file_type: file_type::LEVEL, edition: Edition::Bedrock });
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    let style = run.styles.get(&p(&["Time"])).unwrap();
    assert_eq!(style.extra_text.as_deref(), Some("level"));
    assert_eq!(style.description.as_deref(), Some("edition=BEDROCK"));

    let run2 = mgr.run("", &level_dat(), &Context { file_type: file_type::CHUNK, edition: Edition::Java });
    assert_eq!(run2.styles.get(&p(&["Time"])).unwrap().extra_text.as_deref(), Some("other"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn style_map_shape_is_path_keyed() {
    // The GUI keys rows by TagPath strings; styles must use identical forms,
    // including quoted keys with special characters.
    let root = tmp_root("paths");
    let mgr = Manager::open(&root);
    install_src(&mgr, "paths", r#"
local S = {}
S.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
S.recursive = true
function S:recursion(root_tag, current)
  current:setExtraText("seen")
end
return { S }
"#);
    let doc = Tag::Compound(vec![
        ("plain".into(), Tag::Int(1)),
        ("weird key:with:colons".into(), Tag::Compound(vec![("inner".into(), Tag::Int(2))])),
        ("list".into(), Tag::List(vec![Tag::Compound(vec![("deep".into(), Tag::Byte(3))])])),
    ]);
    let run = mgr.run("", &doc, &ctx());
    let expected: HashMap<String, RowStyle> = run.styles.clone();
    assert_eq!(expected.len(), 6, "{:?}", expected.keys().collect::<Vec<_>>());
    assert!(expected.contains_key(&p(&["plain"])));
    assert!(expected.contains_key(&p(&["weird key:with:colons"])));
    assert!(expected.contains_key(&p(&["weird key:with:colons", "inner"])));
    assert!(expected.contains_key(&p(&["list", "[0]", "deep"])));
    let _ = std::fs::remove_dir_all(&root);
}
