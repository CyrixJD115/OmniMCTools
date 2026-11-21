-- Chest Explorer — the OMT-authored example of the extension platform.
--
-- Shows the three new entry kinds:
--   * EXTENSION_TYPE.TOOL        a tab of its own (sidebar), with a UI built
--                                from `ui.*` and events in `on_event`;
--                                `map = true` also makes it a Map Viewer panel
--   * EXTENSION_TYPE.NBT_ACTION  an action on the NBT editor's selected tag
-- and the `omt` library: worlds (containers, players, level.dat, writes),
-- items, NBT/SNBT, clipboard, notifications, storage, progress.

local DIMS = { "Overworld", "Nether", "End" }

local function dim_id(name)
  for i, n in ipairs(DIMS) do if n == name then return i - 1 end end
  return 0
end

-- Spawn point from level.dat (Java keeps it under Data, Bedrock at the root).
local function spawn(world)
  local lvl = world and world:level()
  if not lvl then return 0, 0 end
  local data = lvl:find("Data") or lvl
  return data:getInt("SpawnX") or 0, data:getInt("SpawnZ") or 0
end

local function total(items, needle)
  local n = 0
  for _, it in ipairs(items) do
    if needle == "" or it.id:find(needle, 1, true) then n = n + it.count end
    for _, inner in ipairs(it.contents) do
      if needle == "" or inner.id:find(needle, 1, true) then n = n + inner.count end
    end
  end
  return n
end

local function item_at(items, slot)
  for _, it in ipairs(items) do if it.slot == slot then return it end end
end

---------------------------------------------------------------------------
-- Tool: Chest Explorer
---------------------------------------------------------------------------
local Explorer = {
  type = EXTENSION_TYPE.TOOL,
  id = "chest-explorer",
  name = "Chest Explorer",
  icon = "chest",
  description = "Find chests, barrels, shulkers and hoppers; search their items; open them as chest menus.",
  map = true,
}

function Explorer:init(ctx)
  self.x, self.z = spawn(ctx.world)
  self.radius = omt.storage.get("radius") or 64
  self.dim = "Overworld"
  self.search = ""
  self.found = {}
  self.rows = {}
  self.sel = nil
  self.slot = nil
  self.inner = nil
  if ctx.world then self:scan(ctx) end
end

function Explorer:scan(ctx)
  local r = self.radius
  local all = ctx.world:containers(dim_id(self.dim), self.x - r, self.z - r, self.x + r, self.z + r)
  -- Keep containers holding the searched item (or all when no search).
  self.found = {}
  for _, c in ipairs(all) do
    if self.search == "" or total(c.items, self.search) > 0 then
      table.insert(self.found, c)
    end
  end
  table.sort(self.found, function(a, b)
    local da = (a.x - self.x) ^ 2 + (a.z - self.z) ^ 2
    local db = (b.x - self.x) ^ 2 + (b.z - self.z) ^ 2
    return da < db
  end)
  self.sel = #self.found > 0 and 1 or nil
  self.slot, self.inner = nil, nil
  local n = 0
  for _, c in ipairs(self.found) do n = n + total(c.items, self.search) end
  if self.search ~= "" then
    omt.notify(string.format("%d × %s in %d container(s)", n, self.search, #self.found))
  end
end

function Explorer:on_event(ev, ctx)
  if ev.kind == "map_click" then
    -- Clicking the map re-centres the search there.
    self.x, self.z, self.dim = ev.x, ev.z, DIMS[ev.dim + 1] or "Overworld"
    self:scan(ctx)
  elseif ev.id == "x" and ev.kind ~= "click" then self.x = tonumber(ev.value) or self.x
  elseif ev.id == "z" and ev.kind ~= "click" then self.z = tonumber(ev.value) or self.z
  elseif ev.id == "search" then
    self.search = ev.value or ""
    if ev.kind == "submit" then self:scan(ctx) end
  elseif ev.id == "radius" then
    self.radius = math.floor(ev.value)
    omt.storage.set("radius", self.radius)
  elseif ev.id == "dim" then self.dim = ev.value
  elseif ev.id == "spawn" then self.x, self.z = spawn(ctx.world); self:scan(ctx)
  elseif ev.id == "scan" then self:scan(ctx)
  elseif ev.id == "list" then self.sel, self.slot, self.inner = ev.row, nil, nil
  elseif ev.id == "chest" then
    self.slot = ev.slot
    local it = item_at(self.found[self.sel].items, ev.slot)
    self.inner = (it and #it.contents > 0) and it or nil
  elseif ev.id == "tp" then
    local c = self.found[self.sel]
    omt.clipboard(string.format("/tp @s %d %d %d", c.x, c.y + 1, c.z))
    omt.notify("copied a /tp command")
  elseif ev.id == "remove" or ev.id == "fill" then
    -- Edit the container in the world (editable sessions; Save to keep).
    local c = self.found[self.sel]
    local items = c.tag:find("Items")
    local changed = false
    for i = items.childCount - 1, 0, -1 do
      local it = items:child(i)
      if (it:getByte("Slot") or -1) == self.slot then
        changed = true
        if ev.id == "remove" then
          items:removeChild(i)
        elseif it:getInt("count") then
          it:setInt("count", 64)
        else
          it:setByte("Count", 64)
        end
      end
    end
    if not changed then
      omt.notify("that slot is empty")
      return
    end
    ctx.world:set_block_entity(dim_id(self.dim), c.x, c.y, c.z, c.tag)
    omt.notify(ev.id == "remove" and "item removed — save the session to keep it" or "stack filled to 64 — save the session to keep it")
    local keep = { x = c.x, y = c.y, z = c.z }
    self:scan(ctx)
    -- Stay on the edited container.
    for i, f in ipairs(self.found) do
      if f.x == keep.x and f.y == keep.y and f.z == keep.z then self.sel = i end
    end
  end
end

function Explorer:view(ctx)
  if not ctx.world then
    return ui.card{ title = "No world", ui.text("Open a world (Map Viewer, NBT Editor…) first, then open Chest Explorer from the sidebar.") }
  end
  local rows = {}
  for i, c in ipairs(self.found) do
    rows[i] = { string.format("%d %d %d", c.x, c.y, c.z), omt.items.name(c.kind), #c.items, total(c.items, self.search) }
  end
  local controls = ui.row{
    ui.text("X"), ui.input{ id = "x", value = self.x, width = 90 },
    ui.text("Z"), ui.input{ id = "z", value = self.z, width = 90 },
    ui.select{ id = "dim", options = DIMS, value = self.dim },
    ui.slider{ id = "radius", label = "Radius", min = 16, max = 512, step = 16, value = self.radius },
    ui.button{ id = "spawn", label = "Spawn", icon = "spawn" },
  }
  local search = ui.row{
    ui.input{ id = "search", value = self.search, placeholder = "item id contains… (e.g. diamond) — Enter to search" },
    ui.button{ id = "scan", label = "Scan", style = "primary", icon = "search" },
  }
  local list = ui.table{ id = "list", columns = { "Position", "Container", "Stacks", self.search == "" and "Items" or ("× " .. self.search) }, rows = rows, selected = self.sel, height = 260 }
  local detail = ui.text{ text = #self.found == 0 and "No containers here — try a bigger radius or another spot (click the map in the Map Viewer)." or "", color = "#9aa0a6" }
  local c = self.sel and self.found[self.sel]
  if c then
    local it = self.slot and item_at(c.items, self.slot)
    detail = ui.column{
      ui.row{
        ui.chest{ id = "chest", title = c.kind, rows = c.rows, cols = c.cols, items = c.items, selected = self.slot },
        self.inner and ui.chest{ id = "inner", title = (self.inner.label or omt.items.name(self.inner.id)) .. " contents", rows = 3, cols = 9, items = self.inner.contents } or nil,
      },
      ui.row{
        ui.button{ id = "tp", label = "Copy /tp", icon = "target" },
        it and ctx.world.writable and ui.button{ id = "fill", label = "Fill stack to 64" } or nil,
        it and ctx.world.writable and ui.button{ id = "remove", label = "Remove item", style = "danger" } or nil,
        not ctx.world.writable and ui.text{ text = "read-only here — open the world in the NBT Editor to edit containers", color = "#9aa0a6", size = 11 } or nil,
      },
      it and ui.card{ title = (it.label or omt.items.name(it.id)) .. " ×" .. it.count, ui.code(omt.nbt.snbt(c.tag:find("Items"):child(self:index_of(c, it.slot)), true)) } or nil,
    }
  end
  return ui.column{ controls, search, ui.text{ text = #self.found .. " container(s) within " .. self.radius .. " blocks of " .. self.x .. ", " .. self.z, size = 11, color = "#9aa0a6" }, list, detail }
end

-- 0-based child index of the item in `slot` (for its SNBT).
function Explorer:index_of(c, slot)
  local items = c.tag:find("Items")
  for i = 0, items.childCount - 1 do
    if (items:child(i):getByte("Slot") or -1) == slot then return i end
  end
  return 0
end

---------------------------------------------------------------------------
-- Tool: Player Inventories
---------------------------------------------------------------------------
local Players = {
  type = EXTENSION_TYPE.TOOL,
  id = "players",
  name = "Player Inventories",
  icon = "player",
  description = "Every player's inventory, armor and ender chest as chest menus.",
}

function Players:init(ctx)
  self.players = ctx.world and ctx.world:players() or {}
  self.sel = #self.players > 0 and 1 or nil
end

function Players:on_event(ev)
  if ev.id == "who" then self.sel = ev.row end
end

function Players:view(ctx)
  if #self.players == 0 then
    return ui.text(ctx.world and "No player data in this world." or "Open a world first.")
  end
  local rows = {}
  for i, p in ipairs(self.players) do rows[i] = { p.id } end
  local p = self.players[self.sel].tag
  local inv = omt.items.read(p:find("Inventory") or omt.nbt.new{})
  local main, armor = {}, {}
  for _, it in ipairs(inv) do
    -- Java armor sits in slots 100–103, the offhand in -106.
    if it.slot >= 100 or it.slot < 0 then
      it.slot = it.slot >= 100 and (103 - it.slot) or 4
      table.insert(armor, it)
    else
      table.insert(main, it)
    end
  end
  for _, it in ipairs(omt.items.read(p:find("Armor") or omt.nbt.new{})) do table.insert(armor, it) end
  return ui.row{
    ui.table{ id = "who", columns = { "Player" }, rows = rows, selected = self.sel, height = 300 },
    ui.column{
      ui.chest{ id = "inv", title = "Inventory", rows = 4, cols = 9, items = main },
      ui.chest{ id = "armor", title = "Armor & offhand", rows = 1, cols = 5, items = armor },
      ui.chest{ id = "ender", title = "Ender chest", rows = 3, cols = 9, items = omt.items.read(p:find("EnderItems") or omt.nbt.new{}) },
    },
  }
end

---------------------------------------------------------------------------
-- NBT action: Open as chest
---------------------------------------------------------------------------
local OpenAsChest = {
  type = EXTENSION_TYPE.NBT_ACTION,
  id = "open-as-chest",
  name = "Open as chest",
  names = { "Items", "Inventory", "EnderItems", "Armor" },
  types = { TYPE.LIST },
}

function OpenAsChest:on_event(ev, ctx)
  if ev.id == "grid" then self.slot = ev.slot end
  if ev.id == "sort" then
    -- Re-number slots by item id (the editor gets the edit as one undo step).
    local list = ctx.node
    local entries = {}
    for i = 0, list.childCount - 1 do table.insert(entries, list:child(i):clone()) end
    table.sort(entries, function(a, b) return (a:getString("id") or a:getString("Name") or "") < (b:getString("id") or b:getString("Name") or "") end)
    list:clear()
    for i, e in ipairs(entries) do
      e:setByte("Slot", i - 1)
      list:addChild(e)
    end
  end
end

function OpenAsChest:view(ctx)
  local items = omt.items.read(ctx.node)
  local rows = math.max(3, math.ceil((#items + 1) / 9))
  local it = self.slot and item_at(items, self.slot)
  return ui.column{
    ui.chest{ id = "grid", title = "Items", rows = math.min(rows, 6), cols = 9, items = items, selected = self.slot },
    ui.row{ ui.button{ id = "sort", label = "Sort by id" }, ui.text{ text = #items .. " stack(s)", color = "#9aa0a6" } },
    it and ui.text((it.label or omt.items.name(it.id)) .. " ×" .. it.count .. (#it.lore > 0 and (" — " .. table.concat(it.lore, ", ")) or "")) or nil,
  }
end

return { Explorer, Players, OpenAsChest }
