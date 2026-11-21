//! Builders for the [`Node`] UI tree. All take `&str`/values and return a
//! `Node`; children collections take anything iterable. Mirrors the Lua
//! `ui.*` helpers.

pub use omt_plugin_abi::{ButtonStyle, Item, Node, Pick};

/// A vertical stack.
pub fn column(children: impl IntoIterator<Item = Node>) -> Node {
    Node::Column { children: children.into_iter().collect(), spacing: 8.0 }
}

/// A horizontal row (wraps).
pub fn row(children: impl IntoIterator<Item = Node>) -> Node {
    Node::Row { children: children.into_iter().collect(), spacing: 8.0 }
}

/// A titled panel.
pub fn card(title: impl Into<String>, children: impl IntoIterator<Item = Node>) -> Node {
    Node::Card { title: Some(title.into()), children: children.into_iter().collect() }
}

/// A scroll area with a pixel height.
pub fn scroll(height: f32, child: Node) -> Node {
    Node::Scroll { child: Box::new(child), height: Some(height) }
}

/// Plain text.
pub fn text(s: impl Into<String>) -> Node {
    Node::Text { text: s.into(), size: 13.0, color: None, bold: false, mono: false }
}

/// Muted/sized/colored text (`color` = `Some((r,g,b))`).
pub fn text_styled(s: impl Into<String>, size: f32, color: Option<(u8, u8, u8)>, bold: bool) -> Node {
    Node::Text { text: s.into(), size, color, bold, mono: false }
}

pub fn heading(s: impl Into<String>) -> Node {
    Node::Heading { text: s.into() }
}

pub fn separator() -> Node {
    Node::Separator
}

pub fn space(size: f32) -> Node {
    Node::Space { size }
}

/// A monospace block (SNBT, reports).
pub fn code(s: impl Into<String>) -> Node {
    Node::Code { text: s.into() }
}

/// A button (`on_event` gets a `Click` with this `id`).
pub fn button(id: impl Into<String>, label: impl Into<String>) -> Button {
    Button(Node::Button { id: id.into(), label: label.into(), style: ButtonStyle::Standard, pick: Pick::None, enabled: true, icon: None })
}

/// Builder returned by [`button`].
pub struct Button(Node);

impl Button {
    pub fn style(mut self, s: ButtonStyle) -> Self {
        if let Node::Button { style, .. } = &mut self.0 {
            *style = s;
        }
        self
    }
    /// Ask for a path first (`Pick::File`/`Folder`/`SaveFile`/`World`); the
    /// event is `Picked` with `ev.path`.
    pub fn pick(mut self, p: Pick) -> Self {
        if let Node::Button { pick, .. } = &mut self.0 {
            *pick = p;
        }
        self
    }
    pub fn icon(mut self, name: impl Into<String>) -> Self {
        if let Node::Button { icon, .. } = &mut self.0 {
            *icon = Some(name.into());
        }
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        if let Node::Button { enabled, .. } = &mut self.0 {
            *enabled = on;
        }
        self
    }
    pub fn build(self) -> Node {
        self.0
    }
}

impl From<Button> for Node {
    fn from(b: Button) -> Node {
        b.0
    }
}

/// A text input (`Input` every key, `Submit` on Enter).
pub fn input(id: impl Into<String>, value: impl Into<String>, placeholder: impl Into<String>) -> Node {
    Node::Input { id: id.into(), value: value.into(), placeholder: placeholder.into(), width: None }
}

pub fn toggle(id: impl Into<String>, label: impl Into<String>, value: bool) -> Node {
    Node::Toggle { id: id.into(), label: label.into(), value }
}

pub fn slider(id: impl Into<String>, label: impl Into<String>, min: f64, max: f64, step: f64, value: f64) -> Node {
    Node::Slider { id: id.into(), min, max, step, value, label: label.into() }
}

pub fn select(id: impl Into<String>, options: impl IntoIterator<Item = String>, value: Option<String>) -> Node {
    Node::Select { id: id.into(), options: options.into_iter().collect(), value, placeholder: String::new() }
}

/// A clickable table (`rows` are 1-based in the `Row` event).
pub fn table(id: impl Into<String>, columns: impl IntoIterator<Item = String>, rows: Vec<Vec<String>>, selected: Option<usize>) -> Node {
    Node::Table { id: id.into(), columns: columns.into_iter().collect(), rows, selected, height: None }
}

/// A Minecraft container grid of [`Item`]s (`Slot` event on click).
pub fn chest(id: impl Into<String>, title: impl Into<String>, rows: usize, cols: usize, items: Vec<Item>) -> Node {
    Node::Chest { id: id.into(), title: title.into(), rows, cols, items, selected: None }
}

/// A block/item icon from the active textures.
pub fn block(name: impl Into<String>, size: f32) -> Node {
    Node::Block { name: name.into(), size }
}

pub fn progress(value: f32, label: impl Into<String>) -> Node {
    Node::Progress { value, label: label.into() }
}

/// A small coloured pill (`color` = `Some((r,g,b))`).
pub fn badge(text: impl Into<String>, color: Option<(u8, u8, u8)>) -> Node {
    Node::Badge { text: text.into(), color }
}

/// An image file (PNG/JPG…) at `size` px.
pub fn image(path: impl Into<String>, size: f32) -> Node {
    Node::Image { path: path.into(), size }
}

/// A numeric field with − / + steppers (`Input` live, `Submit` on step).
pub fn number(id: impl Into<String>, value: f64, min: f64, max: f64, step: f64) -> Node {
    Node::Number { id: id.into(), value, min, max, step }
}
