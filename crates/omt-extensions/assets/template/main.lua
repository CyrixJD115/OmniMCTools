-- __NAME__ — a new OmniMCTools extension.
--
-- Edit this file, then press "Reload" on the tool's tab (or "Reload all" on
-- the Extensions page) to see your changes. The full guide with every UI
-- node and `omt.*` call: crates/omt-extensions/EXTENSIONS.md (also in the
-- app's source tree), and the built-in Chest Explorer is a bigger example.

local Tool = {
  type = EXTENSION_TYPE.TOOL,   -- a tab of its own in the sidebar
  id = "main",
  name = "__NAME__",
  icon = "sparkles",
  description = "My first tool.",
  map = true,                   -- also usable from the Map Viewer's "Extension tools" panel
}

-- Once, when the tab opens. Keep your state on `self`.
function Tool:init(ctx)
  self.clicks = omt.storage.get("clicks") or 0   -- remembered across sessions
  self.note = ""
  self.picked = nil
  -- Heavy work belongs in init / on_event, not view (view runs after every event).
  self.chunks = ctx.world and #ctx.world:chunks(0) or 0
end

-- Every button click, input, slot click or Map Viewer click.
function Tool:on_event(ev, ctx)
  if ev.kind == "click" and ev.id == "hello" then
    self.clicks = self.clicks + 1
    omt.storage.set("clicks", self.clicks)
    omt.notify("hello #" .. self.clicks)
  elseif ev.kind == "input" and ev.id == "note" then
    self.note = ev.value
  elseif ev.kind == "map_click" then
    -- Which block is there, and is it a container?
    local name = ctx.world and ctx.world:block(ev.dim, ev.x, 64, ev.z)
    self.picked = string.format("%d, %d → %s", ev.x, ev.z, name or "?")
  end
end

-- After every callback: describe the screen.
function Tool:view(ctx)
  local w = ctx.world
  local info = w and ui.card{
    title = w.name,
    ui.text(w.edition .. (w.writable and " · editable" or " · read-only")),
    ui.text(self.chunks .. " chunks in the Overworld"),
  } or ui.text("Open a world (Map Viewer, NBT Editor…) to use world calls.")
  return ui.column{
    ui.heading("Hello from __NAME__"),
    info,
    ui.row{
      ui.button{ id = "hello", label = "Say hello (" .. self.clicks .. ")", style = "primary", icon = "sparkles" },
      ui.input{ id = "note", value = self.note, placeholder = "type something…", width = 220 },
    },
    self.note ~= "" and ui.text("You typed: " .. self.note) or nil,
    self.picked and ui.text("Map click: " .. self.picked) or nil,
  }
end

return { Tool }
