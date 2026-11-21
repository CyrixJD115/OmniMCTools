//! Convert the plugin wire types ([`omt_plugin_abi`]) to the app's tool
//! types ([`omt_extensions::ui`]) and back.

use omt_extensions::ui::{ButtonStyle, ItemSlot, Pick, ToolEvent, UiNode};
use omt_plugin_abi as abi;

fn style(s: abi::ButtonStyle) -> ButtonStyle {
    match s {
        abi::ButtonStyle::Standard => ButtonStyle::Standard,
        abi::ButtonStyle::Primary => ButtonStyle::Primary,
        abi::ButtonStyle::Danger => ButtonStyle::Danger,
        abi::ButtonStyle::Ghost => ButtonStyle::Ghost,
    }
}

fn pick(p: abi::Pick) -> Pick {
    match p {
        abi::Pick::None => Pick::None,
        abi::Pick::File => Pick::File,
        abi::Pick::Folder => Pick::Folder,
        abi::Pick::SaveFile => Pick::SaveFile,
        abi::Pick::World => Pick::World,
    }
}

fn slot(it: abi::Item) -> ItemSlot {
    ItemSlot { slot: it.slot, id: it.id, count: it.count, label: it.label, lore: it.lore, glint: it.glint }
}

/// A plugin UI node → the app's `UiNode`.
pub fn node(n: abi::Node) -> UiNode {
    match n {
        abi::Node::Column { children, spacing } => UiNode::Column { children: children.into_iter().map(node).collect(), spacing },
        abi::Node::Row { children, spacing } => UiNode::Row { children: children.into_iter().map(node).collect(), spacing },
        abi::Node::Card { title, children } => UiNode::Card { title, children: children.into_iter().map(node).collect() },
        abi::Node::Scroll { child, height } => UiNode::Scroll { child: Box::new(node(*child)), height },
        abi::Node::Text { text, size, color, bold, mono } => UiNode::Text { text, size, color, bold, mono },
        abi::Node::Heading { text } => UiNode::Heading { text },
        abi::Node::Separator => UiNode::Separator,
        abi::Node::Space { size } => UiNode::Space { size },
        abi::Node::Button { id, label, style: s, pick: p, enabled, icon } => UiNode::Button { id, label, style: style(s), pick: pick(p), enabled, icon },
        abi::Node::Input { id, value, placeholder, width } => UiNode::Input { id, value, placeholder, width },
        abi::Node::Toggle { id, label, value } => UiNode::Toggle { id, label, value },
        abi::Node::Slider { id, min, max, step, value, label } => UiNode::Slider { id, min, max, step, value, label },
        abi::Node::Select { id, options, value, placeholder } => UiNode::Select { id, options, value, placeholder },
        abi::Node::Table { id, columns, rows, selected, height } => UiNode::Table { id, columns, rows, selected, height },
        abi::Node::Chest { id, title, rows, cols, items, selected } => UiNode::Chest { id, title, rows, cols, items: items.into_iter().map(slot).collect(), selected },
        abi::Node::Block { name, size } => UiNode::Block { name, size },
        abi::Node::Progress { value, label } => UiNode::Progress { value, label },
        abi::Node::Code { text } => UiNode::Code { text },
    }
}

/// A UI event → the wire event.
pub fn event(e: ToolEvent) -> abi::Event {
    match e {
        ToolEvent::Click { id } => abi::Event::Click { id },
        ToolEvent::Input { id, value } => abi::Event::Input { id, value },
        ToolEvent::Submit { id, value } => abi::Event::Submit { id, value },
        ToolEvent::Toggle { id, value } => abi::Event::Toggle { id, value },
        ToolEvent::Slider { id, value } => abi::Event::Slider { id, value },
        ToolEvent::Select { id, value } => abi::Event::Select { id, value },
        ToolEvent::Row { id, row } => abi::Event::Row { id, row },
        ToolEvent::Slot { id, slot } => abi::Event::Slot { id, slot },
        ToolEvent::Picked { id, path } => abi::Event::Picked { id, path },
        ToolEvent::MapClick { x, y, z, dim } => abi::Event::MapClick { x, y, z, dim },
    }
}
