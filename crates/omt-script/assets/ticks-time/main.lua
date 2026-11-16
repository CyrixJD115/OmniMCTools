-- Tick Clock — the OMT-authored example extension.
--
-- Demonstrates the NBT-editor-style protocol: this file returns a table of
-- sub-extensions; the ones flagged EXTENSION_TYPE.NBT_EDITOR_STYLE get their
-- `main` called once per document and their `recursion` called at every tag
-- during the host's depth-first walk. Styling is done with the OMT-defined
-- setExtraText / setDescription / setColor methods.
--
-- Minecraft time facts used below:
--   * a full day is 24000 ticks; tick 0 of a day is 06:00 in-game
--   * 1000 ticks = 1 in-game hour, so minutes = (ticks % 1000) * 60 / 1000
--   * the game simulates 20 ticks per real-time second

local IN_GAME_KEYS = {
  time = "world age and time of day",
  daytime = "time of day",
}

local function in_game_clock(ticks)
  if ticks < 0 then return nil end
  local day = math.floor(ticks / 24000)
  local tod = ticks % 24000
  local hours = (math.floor(tod / 1000) + 6) % 24
  local minutes = math.floor((tod % 1000) * 60 / 1000)
  return string.format("day %d, %02d:%02d in-game", day, hours, minutes)
end

local function real_time(ticks)
  if ticks < 0 then return nil end
  local seconds = math.floor(ticks / 20)
  local h = math.floor(seconds / 3600)
  local m = math.floor((seconds % 3600) / 60)
  local s = seconds % 60
  if h > 0 then
    return string.format("%dh %02dm %02ds played", h, m, s)
  end
  return string.format("%dm %02ds played", m, s)
end

local TickClock = {}

TickClock.type = EXTENSION_TYPE.NBT_EDITOR_STYLE
TickClock.recursive = true

function TickClock:main(root, context)
  -- Nothing to do up-front; everything happens in the walk.
end

function TickClock:recursion(root, current, context)
  if current.type ~= TYPE.LONG and current.type ~= TYPE.INT then
    return
  end
  local key = string.lower(current.name or "")
  local ticks = current.value
  if IN_GAME_KEYS[key] then
    local text = in_game_clock(ticks)
    if text then
      current:setExtraText(text)
      current:setDescription("Tick Clock: " .. key .. " — " .. IN_GAME_KEYS[key]
        .. " (24000 ticks per day, day starts at 06:00).")
      current:setColor("#7fb069")
    end
  elseif key == "inhabitedtime" then
    local text = real_time(ticks)
    if text then
      current:setExtraText(text)
      current:setDescription("Tick Clock: InhabitedTime — how long players have spent near this chunk (20 ticks per second).")
      current:setColor("#6aa9c9")
    end
  end
end

return { TickClock }
