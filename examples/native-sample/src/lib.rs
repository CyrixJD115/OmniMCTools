//! Example OmniMCTools **native plugin** (compiled `cdylib`). Build it:
//!
//! ```text
//! cargo build -p omt-native-sample --release
//! ```
//!
//! then drop the resulting `libomt_native_sample.so` (/.dll/.dylib) into a
//! folder under OMT's extensions directory and enable it — a native plugin
//! folder is any folder holding one dynamic library.
//!
//! It adds a **Containers** tool (also a Map Viewer panel) that finds
//! nearby chests, shows one as a Minecraft chest grid and can fill a stack
//! in an editable session, and a **Fill stacks** NBT action that sets every
//! stack in an `Items`/`Inventory` list to 64.

use omt_plugin::prelude::*;

/// Tool: browse containers near a point.
struct Containers {
    cx: i32,
    cz: i32,
    dim: u8,
    found: Vec<omt_plugin::Item>,
    rows: Vec<ContainerRow>,
    sel: usize,
    slot: Option<i32>,
}

/// One found container (kept so a click can write it back).
struct ContainerRow {
    x: i32,
    y: i32,
    z: i32,
    kind: String,
    chest_rows: usize,
    cols: usize,
    snbt: String,
    items: Vec<omt_plugin::Item>,
}

impl Containers {
    fn scan(&mut self, host: &Host) {
        self.rows.clear();
        self.found.clear();
        self.sel = 0;
        self.slot = None;
        match host.containers(self.dim, self.cx - 64, self.cz - 64, self.cx + 64, self.cz + 64) {
            Ok(cs) => {
                for c in cs {
                    self.rows.push(ContainerRow { x: c.x, y: c.y, z: c.z, kind: c.kind, chest_rows: c.rows, cols: c.cols, snbt: c.snbt, items: c.items });
                }
                host.notify(format!("{} container(s) near {}, {}", self.rows.len(), self.cx, self.cz));
            }
            Err(e) => host.notify(format!("scan failed: {e}")),
        }
    }
}

impl Instance for Containers {
    fn on_event(&mut self, ev: &Event, host: &Host, ctx: &Ctx) {
        match ev {
            Event::Input { id, value } | Event::Submit { id, value } if id == "x" => self.cx = value.parse().unwrap_or(self.cx),
            Event::Input { id, value } | Event::Submit { id, value } if id == "z" => self.cz = value.parse().unwrap_or(self.cz),
            Event::Click { id } if id == "scan" => self.scan(host),
            Event::MapClick { x, z, dim, .. } => {
                self.cx = *x;
                self.cz = *z;
                self.dim = *dim;
                self.scan(host);
            }
            Event::Row { id, row } if id == "list" => {
                self.sel = row.saturating_sub(1);
                self.slot = None;
            }
            Event::Slot { id, slot } if id == "chest" => self.slot = Some(*slot),
            Event::Click { id } if id == "fill" => self.fill_slot(host, ctx),
            _ => {}
        }
    }

    fn view(&mut self, _host: &Host, ctx: &Ctx) -> Node {
        let Some(w) = ctx.world() else {
            return ui::text("Open a world (Map Viewer, NBT Editor…) first.");
        };
        let rows: Vec<Vec<String>> = self.rows.iter().map(|c| vec![format!("{} {} {}", c.x, c.y, c.z), c.kind.clone(), c.items.len().to_string()]).collect();
        let mut col = vec![
            ui::heading("Containers"),
            ui::row([ui::text("X"), ui::input("x", self.cx.to_string(), ""), ui::text("Z"), ui::input("z", self.cz.to_string(), ""), ui::button("scan", "Scan").style(ButtonStyle::Primary).icon("search").into()]),
            ui::text_styled(format!("{} within 64 of {}, {} — click the map to re-centre", self.rows.len(), self.cx, self.cz), 11.0, Some((0x9a, 0xa0, 0xa6)), false),
            ui::table("list", ["Position".into(), "Container".into(), "Items".into()], rows, Some(self.sel + 1)),
        ];
        if let Some(c) = self.rows.get(self.sel) {
            col.push(ui::chest("chest", c.kind.clone(), c.chest_rows, c.cols, c.items.clone()));
            if self.slot.is_some() && w.writable {
                col.push(ui::button("fill", "Fill the selected slot to 64").into());
            } else if !w.writable {
                col.push(ui::text_styled("read-only here — open in the NBT Editor to edit", 11.0, Some((0x9a, 0xa0, 0xa6)), false));
            }
        }
        ui::column(col)
    }
}

impl Containers {
    fn fill_slot(&mut self, host: &Host, _ctx: &Ctx) {
        let (Some(slot), Some(c)) = (self.slot, self.rows.get(self.sel)) else { return };
        // Rebuild the container tag with the chosen slot's stack at count 64.
        // Items travel as SNBT; we edit it with a tiny textual splice via the
        // host's items round-trip, then write the whole block entity back.
        let items = host.items_read(&c.snbt);
        let Some(it) = items.iter().find(|i| i.slot == slot) else {
            host.notify("that slot is empty");
            return;
        };
        // Compose a fresh Items list: keep all, override this slot's count.
        let edition_java = c.snbt.contains("count:");
        let mut parts: Vec<String> = Vec::new();
        for i in &items {
            let count = if i.slot == slot { 64 } else { i.count };
            if let Some(stack) = host.item_make(&i.id, count, i.slot, edition_java) {
                parts.push(stack);
            }
        }
        let be = replace_items(&c.snbt, &parts.join(","));
        match host.set_block_entity(self.dim, c.x, c.y, c.z, &be) {
            Ok(()) => {
                host.notify(format!("filled {} to 64 — Save the session to keep it", it.id));
                self.scan(host);
            }
            Err(e) => host.notify(format!("write failed: {e}")),
        }
    }
}

/// Replace the `Items:[…]` list in a block-entity SNBT string with `inner`.
fn replace_items(snbt: &str, inner: &str) -> String {
    if let Some(start) = snbt.find("Items:[") {
        let open = start + "Items:[".len();
        // Find the matching close bracket.
        let mut depth = 1;
        let bytes = snbt.as_bytes();
        let mut i = open;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        format!("{}{inner}{}", &snbt[..open], &snbt[i - 1..])
    } else {
        // No Items yet: add one before the closing brace.
        match snbt.rfind('}') {
            Some(b) => format!("{},Items:[{inner}]{}", &snbt[..b], &snbt[b..]),
            None => snbt.to_string(),
        }
    }
}

/// NBT action: set every stack in the selected list to 64.
struct FillStacks {
    done: Option<usize>,
}

impl Instance for FillStacks {
    fn on_event(&mut self, ev: &Event, host: &Host, ctx: &Ctx) {
        if let Event::Click { id } = ev
            && id == "fill"
            && let Some(node) = ctx.node()
        {
            let items = host.items_read(node);
            let java = node.contains("count:");
            let parts: Vec<String> = items.iter().filter_map(|i| host.item_make(&i.id, 64, i.slot, java)).collect();
            // The selected tag IS the Items list, so replace it wholesale.
            host.replace_node(format!("[{}]", parts.join(",")));
            self.done = Some(parts.len());
            host.notify(format!("set {} stack(s) to 64", parts.len()));
        }
    }

    fn view(&mut self, host: &Host, ctx: &Ctx) -> Node {
        let items = ctx.node().map(|n| host.items_read(n)).unwrap_or_default();
        let rows = items.len().div_ceil(9).clamp(3, 6);
        ui::column([
            ui::chest("grid", "Items", rows, 9, items),
            ui::row([ui::button("fill", "Set every stack to 64").style(ButtonStyle::Primary).into(), ui::text(self.done.map_or_else(String::new, |n| format!("done: {n} stacks")))]),
        ])
    }
}

fn plugin() -> Plugin {
    Plugin::new("Native Sample")
        .version("1.0.0")
        .author("OmniMCTools")
        .describe("Example native (Rust) plugin: a container browser and a Fill-stacks NBT action.")
        .tool(ToolDef::new("containers", "Containers").icon("box").map(true).describe("Find nearby chests and open them as chest menus."), |_ctx| Box::new(Containers { cx: 0, cz: 0, dim: 0, found: Vec::new(), rows: Vec::new(), sel: 0, slot: None }))
        .action(ActionDef::new("fill-stacks", "Fill stacks to 64").names(&["Items", "Inventory", "EnderItems"]).types(&[types::LIST]), |_ctx| Box::new(FillStacks { done: None }))
}

export_plugin!(plugin);
