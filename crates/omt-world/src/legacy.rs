//! Pre-flattening numeric block and biome ids → modern Java names.
//!
//! Java worlds before 1.13 (Anvil `Blocks`/`Add`/`Data` sections, Beta
//! McRegion) and Bedrock worlds before 1.2.13 (sub-chunk versions 0–7,
//! pre-1.0 `LegacyTerrain`) store blocks as `(id, data)` pairs. The map
//! renderer and inspectors work on modern Java names, so these tables map
//! every vanilla id (with the data variants that change a block's look) to
//! its current name. Directional / state-only data bits are ignored.
//!
//! Facts from the public block-id lists (the Minecraft Wiki "Java Edition
//! data values" / "Bedrock Edition data values" pages); no game code.

const COLORS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];
const WOODS: [&str; 6] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak"];

macro_rules! names {
    ($($s:literal),* $(,)?) => { &[$(concat!("minecraft:", $s)),*] };
}

/// Pick `variants[data % len]` (or the first when `data` is out of range).
fn pick(variants: &'static [&'static str], data: u8) -> &'static str {
    variants.get(data as usize).copied().unwrap_or(variants[0])
}

/// Interned `minecraft:<color>_<suffix>` / `minecraft:<wood>_<suffix>` names.
fn family(prefixes: &'static [&'static str], suffix: &'static str, i: usize) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static NAMES: OnceLock<Mutex<HashMap<(&'static str, &'static str), &'static str>>> = OnceLock::new();
    let p = prefixes.get(i).copied().unwrap_or(prefixes[0]);
    let mut m = NAMES.get_or_init(Default::default).lock().expect("legacy names poisoned");
    m.entry((p, suffix)).or_insert_with(|| Box::leak(format!("minecraft:{p}_{suffix}").into_boxed_str()))
}

/// Java Edition 1.12 and earlier: `(id, data)` → modern block name.
pub fn java_block(id: u16, data: u8) -> &'static str {
    let d = data & 0xF;
    match id {
        0 => "minecraft:air",
        1 => pick(names!["stone", "granite", "polished_granite", "diorite", "polished_diorite", "andesite", "polished_andesite"], d),
        2 => "minecraft:grass_block",
        3 => pick(names!["dirt", "coarse_dirt", "podzol"], d),
        4 => "minecraft:cobblestone",
        5 => family(&WOODS, "planks", d as usize),
        6 => family(&WOODS, "sapling", (d & 7) as usize),
        7 => "minecraft:bedrock",
        8 | 9 => "minecraft:water",
        10 | 11 => "minecraft:lava",
        12 => pick(names!["sand", "red_sand"], d),
        13 => "minecraft:gravel",
        14 => "minecraft:gold_ore",
        15 => "minecraft:iron_ore",
        16 => "minecraft:coal_ore",
        17 => family(&WOODS, "log", (d & 3) as usize),
        18 => family(&WOODS, "leaves", (d & 3) as usize),
        19 => pick(names!["sponge", "wet_sponge"], d),
        20 => "minecraft:glass",
        21 => "minecraft:lapis_ore",
        22 => "minecraft:lapis_block",
        23 => "minecraft:dispenser",
        24 => pick(names!["sandstone", "chiseled_sandstone", "cut_sandstone"], d),
        25 => "minecraft:note_block",
        26 => "minecraft:red_bed",
        27 => "minecraft:powered_rail",
        28 => "minecraft:detector_rail",
        29 => "minecraft:sticky_piston",
        30 => "minecraft:cobweb",
        31 => pick(names!["dead_bush", "short_grass", "fern"], d),
        32 => "minecraft:dead_bush",
        33 => "minecraft:piston",
        34 => "minecraft:piston_head",
        35 => family(&COLORS, "wool", d as usize),
        36 => "minecraft:moving_piston",
        37 => "minecraft:dandelion",
        38 => pick(names!["poppy", "blue_orchid", "allium", "azure_bluet", "red_tulip", "orange_tulip", "white_tulip", "pink_tulip", "oxeye_daisy"], d),
        39 => "minecraft:brown_mushroom",
        40 => "minecraft:red_mushroom",
        41 => "minecraft:gold_block",
        42 => "minecraft:iron_block",
        43 | 44 => pick(names!["smooth_stone_slab", "sandstone_slab", "petrified_oak_slab", "cobblestone_slab", "brick_slab", "stone_brick_slab", "nether_brick_slab", "quartz_slab"], d & 7),
        45 => "minecraft:bricks",
        46 => "minecraft:tnt",
        47 => "minecraft:bookshelf",
        48 => "minecraft:mossy_cobblestone",
        49 => "minecraft:obsidian",
        50 => "minecraft:torch",
        51 => "minecraft:fire",
        52 => "minecraft:spawner",
        53 => "minecraft:oak_stairs",
        54 => "minecraft:chest",
        55 => "minecraft:redstone_wire",
        56 => "minecraft:diamond_ore",
        57 => "minecraft:diamond_block",
        58 => "minecraft:crafting_table",
        59 => "minecraft:wheat",
        60 => "minecraft:farmland",
        61 | 62 => "minecraft:furnace",
        63 => "minecraft:oak_sign",
        64 => "minecraft:oak_door",
        65 => "minecraft:ladder",
        66 => "minecraft:rail",
        67 => "minecraft:cobblestone_stairs",
        68 => "minecraft:oak_wall_sign",
        69 => "minecraft:lever",
        70 => "minecraft:stone_pressure_plate",
        71 => "minecraft:iron_door",
        72 => "minecraft:oak_pressure_plate",
        73 | 74 => "minecraft:redstone_ore",
        75 | 76 => "minecraft:redstone_torch",
        77 => "minecraft:stone_button",
        78 => "minecraft:snow",
        79 => "minecraft:ice",
        80 => "minecraft:snow_block",
        81 => "minecraft:cactus",
        82 => "minecraft:clay",
        83 => "minecraft:sugar_cane",
        84 => "minecraft:jukebox",
        85 => "minecraft:oak_fence",
        86 => "minecraft:pumpkin",
        87 => "minecraft:netherrack",
        88 => "minecraft:soul_sand",
        89 => "minecraft:glowstone",
        90 => "minecraft:nether_portal",
        91 => "minecraft:jack_o_lantern",
        92 => "minecraft:cake",
        93 | 94 => "minecraft:repeater",
        95 => family(&COLORS, "stained_glass", d as usize),
        96 => "minecraft:oak_trapdoor",
        97 => pick(names!["infested_stone", "infested_cobblestone", "infested_stone_bricks", "infested_mossy_stone_bricks", "infested_cracked_stone_bricks", "infested_chiseled_stone_bricks"], d),
        98 => pick(names!["stone_bricks", "mossy_stone_bricks", "cracked_stone_bricks", "chiseled_stone_bricks"], d),
        99 => "minecraft:brown_mushroom_block",
        100 => "minecraft:red_mushroom_block",
        101 => "minecraft:iron_bars",
        102 => "minecraft:glass_pane",
        103 => "minecraft:melon",
        104 => "minecraft:pumpkin_stem",
        105 => "minecraft:melon_stem",
        106 => "minecraft:vine",
        107 => "minecraft:oak_fence_gate",
        108 => "minecraft:brick_stairs",
        109 => "minecraft:stone_brick_stairs",
        110 => "minecraft:mycelium",
        111 => "minecraft:lily_pad",
        112 => "minecraft:nether_bricks",
        113 => "minecraft:nether_brick_fence",
        114 => "minecraft:nether_brick_stairs",
        115 => "minecraft:nether_wart",
        116 => "minecraft:enchanting_table",
        117 => "minecraft:brewing_stand",
        118 => "minecraft:cauldron",
        119 => "minecraft:end_portal",
        120 => "minecraft:end_portal_frame",
        121 => "minecraft:end_stone",
        122 => "minecraft:dragon_egg",
        123 | 124 => "minecraft:redstone_lamp",
        125 | 126 => family(&WOODS, "slab", (d & 7) as usize),
        127 => "minecraft:cocoa",
        128 => "minecraft:sandstone_stairs",
        129 => "minecraft:emerald_ore",
        130 => "minecraft:ender_chest",
        131 => "minecraft:tripwire_hook",
        132 => "minecraft:tripwire",
        133 => "minecraft:emerald_block",
        134 => "minecraft:spruce_stairs",
        135 => "minecraft:birch_stairs",
        136 => "minecraft:jungle_stairs",
        137 => "minecraft:command_block",
        138 => "minecraft:beacon",
        139 => pick(names!["cobblestone_wall", "mossy_cobblestone_wall"], d),
        140 => "minecraft:flower_pot",
        141 => "minecraft:carrots",
        142 => "minecraft:potatoes",
        143 => "minecraft:oak_button",
        144 => "minecraft:skeleton_skull",
        145 => "minecraft:anvil",
        146 => "minecraft:trapped_chest",
        147 => "minecraft:light_weighted_pressure_plate",
        148 => "minecraft:heavy_weighted_pressure_plate",
        149 | 150 => "minecraft:comparator",
        151 | 178 => "minecraft:daylight_detector",
        152 => "minecraft:redstone_block",
        153 => "minecraft:nether_quartz_ore",
        154 => "minecraft:hopper",
        155 => pick(names!["quartz_block", "chiseled_quartz_block", "quartz_pillar"], d.min(2)),
        156 => "minecraft:quartz_stairs",
        157 => "minecraft:activator_rail",
        158 => "minecraft:dropper",
        159 => family(&COLORS, "terracotta", d as usize),
        160 => family(&COLORS, "stained_glass_pane", d as usize),
        161 => pick(names!["acacia_leaves", "dark_oak_leaves"], d & 1),
        162 => pick(names!["acacia_log", "dark_oak_log"], d & 1),
        163 => "minecraft:acacia_stairs",
        164 => "minecraft:dark_oak_stairs",
        165 => "minecraft:slime_block",
        166 => "minecraft:barrier",
        167 => "minecraft:iron_trapdoor",
        168 => pick(names!["prismarine", "prismarine_bricks", "dark_prismarine"], d),
        169 => "minecraft:sea_lantern",
        170 => "minecraft:hay_block",
        171 => family(&COLORS, "carpet", d as usize),
        172 => "minecraft:terracotta",
        173 => "minecraft:coal_block",
        174 => "minecraft:packed_ice",
        // Upper halves (data ≥ 8) carry no type: render them as tall grass.
        175 => pick(names!["sunflower", "lilac", "tall_grass", "large_fern", "rose_bush", "peony"], if d >= 8 { 2 } else { d }),
        176 => "minecraft:white_banner",
        177 => "minecraft:white_wall_banner",
        179 => pick(names!["red_sandstone", "chiseled_red_sandstone", "cut_red_sandstone"], d),
        180 => "minecraft:red_sandstone_stairs",
        181 | 182 => "minecraft:red_sandstone_slab",
        183 => "minecraft:spruce_fence_gate",
        184 => "minecraft:birch_fence_gate",
        185 => "minecraft:jungle_fence_gate",
        186 => "minecraft:dark_oak_fence_gate",
        187 => "minecraft:acacia_fence_gate",
        188 => "minecraft:spruce_fence",
        189 => "minecraft:birch_fence",
        190 => "minecraft:jungle_fence",
        191 => "minecraft:dark_oak_fence",
        192 => "minecraft:acacia_fence",
        193 => "minecraft:spruce_door",
        194 => "minecraft:birch_door",
        195 => "minecraft:jungle_door",
        196 => "minecraft:acacia_door",
        197 => "minecraft:dark_oak_door",
        198 => "minecraft:end_rod",
        199 => "minecraft:chorus_plant",
        200 => "minecraft:chorus_flower",
        201 => "minecraft:purpur_block",
        202 => "minecraft:purpur_pillar",
        203 => "minecraft:purpur_stairs",
        204 | 205 => "minecraft:purpur_slab",
        206 => "minecraft:end_stone_bricks",
        207 => "minecraft:beetroots",
        208 => "minecraft:dirt_path",
        209 => "minecraft:end_gateway",
        210 => "minecraft:repeating_command_block",
        211 => "minecraft:chain_command_block",
        212 => "minecraft:frosted_ice",
        213 => "minecraft:magma_block",
        214 => "minecraft:nether_wart_block",
        215 => "minecraft:red_nether_bricks",
        216 => "minecraft:bone_block",
        217 => "minecraft:structure_void",
        218 => "minecraft:observer",
        219..=234 => family(&COLORS, "shulker_box", (id - 219) as usize),
        235..=250 => family(&COLORS, "glazed_terracotta", (id - 235) as usize),
        251 => family(&COLORS, "concrete", d as usize),
        252 => family(&COLORS, "concrete_powder", d as usize),
        255 => "minecraft:structure_block",
        _ => "minecraft:air",
    }
}

/// Bedrock Edition before 1.2.13 (sub-chunk versions 0–7, LegacyTerrain):
/// the classic ids shared with Java plus Bedrock's own assignments.
pub fn bedrock_block(id: u16, data: u8) -> &'static str {
    let d = data & 0xF;
    match id {
        44 | 43 => pick(names!["smooth_stone_slab", "sandstone_slab", "petrified_oak_slab", "cobblestone_slab", "brick_slab", "stone_brick_slab", "quartz_slab", "nether_brick_slab"], d & 7),
        85 => family(&WOODS, "fence", (d % 6) as usize),
        95 => "minecraft:barrier", // invisible bedrock
        125 => "minecraft:dropper",
        126 => "minecraft:activator_rail",
        157 | 158 => family(&WOODS, "slab", (d & 7) as usize),
        181 | 182 => pick(names!["red_sandstone_slab", "purpur_slab", "prismarine_slab", "dark_prismarine_slab", "prismarine_brick_slab", "mossy_cobblestone_slab", "smooth_sandstone_slab", "red_nether_brick_slab"], d & 7),
        188 => "minecraft:repeating_command_block",
        189 => "minecraft:chain_command_block",
        190..=192 => "minecraft:glass",
        198 => "minecraft:dirt_path",
        199 => "minecraft:air", // item frame (a block on Bedrock)
        200 => "minecraft:chorus_flower",
        201 => "minecraft:purpur_block",
        202 => "minecraft:torch",
        203 => "minecraft:purpur_stairs",
        204 => "minecraft:torch",
        205 => "minecraft:shulker_box",
        206 => "minecraft:end_stone_bricks",
        207 => "minecraft:frosted_ice",
        208 => "minecraft:end_rod",
        209 => "minecraft:end_gateway",
        210 | 211 => "minecraft:bedrock", // allow / deny
        212 => "minecraft:bedrock",       // border block
        213 => "minecraft:magma_block",
        214 => "minecraft:nether_wart_block",
        215 => "minecraft:red_nether_bricks",
        216 => "minecraft:bone_block",
        217 => "minecraft:structure_void",
        218 => family(&COLORS, "shulker_box", d as usize),
        219 => "minecraft:purple_glazed_terracotta",
        220..=229 => family(&COLORS, "glazed_terracotta", (id - 220) as usize),
        231..=235 => family(&COLORS, "glazed_terracotta", (id - 231 + 11) as usize),
        236 => family(&COLORS, "concrete", d as usize),
        237 => family(&COLORS, "concrete_powder", d as usize),
        238 | 230 | 242 => "minecraft:air",
        239 => "minecraft:torch",
        240 => "minecraft:chorus_plant",
        241 => family(&COLORS, "stained_glass", d as usize),
        243 => "minecraft:podzol",
        244 => "minecraft:beetroots",
        245 => "minecraft:stonecutter",
        246 => "minecraft:crying_obsidian", // glowing obsidian
        247 => "minecraft:iron_block",     // nether reactor core
        248 | 249 => "minecraft:bedrock",  // update-game blocks
        250 => "minecraft:moving_piston",
        251 => "minecraft:observer",
        252 => "minecraft:structure_block",
        253 => "minecraft:glass",
        254 | 255 => "minecraft:air",
        _ => java_block(id, data),
    }
}

/// Java numeric biome ids (≤ 1.17 chunks; Bedrock legacy `Data2D` uses the
/// same classic ids) → current biome names (1.18 merges applied).
pub fn biome(id: u32) -> &'static str {
    match id {
        0 => "minecraft:ocean",
        1 => "minecraft:plains",
        2 | 17 | 130 => "minecraft:desert",
        3 | 13 | 20 => "minecraft:windswept_hills",
        4 | 18 => "minecraft:forest",
        5 | 19 | 133 => "minecraft:taiga",
        6 | 134 => "minecraft:swamp",
        7 => "minecraft:river",
        8 => "minecraft:nether_wastes",
        9 => "minecraft:the_end",
        10 => "minecraft:frozen_ocean",
        11 => "minecraft:frozen_river",
        12 => "minecraft:snowy_plains",
        14 | 15 => "minecraft:mushroom_fields",
        16 => "minecraft:beach",
        21 | 22 | 149 => "minecraft:jungle",
        23 | 151 => "minecraft:sparse_jungle",
        24 => "minecraft:deep_ocean",
        25 => "minecraft:stony_shore",
        26 => "minecraft:snowy_beach",
        27 | 28 => "minecraft:birch_forest",
        29 | 157 => "minecraft:dark_forest",
        30 | 31 | 158 => "minecraft:snowy_taiga",
        32 | 33 => "minecraft:old_growth_pine_taiga",
        34 => "minecraft:windswept_forest",
        35 => "minecraft:savanna",
        36 => "minecraft:savanna_plateau",
        37 | 39 | 167 => "minecraft:badlands",
        38 | 166 => "minecraft:wooded_badlands",
        40 => "minecraft:small_end_islands",
        41 => "minecraft:end_midlands",
        42 => "minecraft:end_highlands",
        43 => "minecraft:end_barrens",
        44 | 47 => "minecraft:warm_ocean",
        45 => "minecraft:lukewarm_ocean",
        46 => "minecraft:cold_ocean",
        48 => "minecraft:deep_lukewarm_ocean",
        49 => "minecraft:deep_cold_ocean",
        50 => "minecraft:deep_frozen_ocean",
        127 => "minecraft:the_void",
        129 => "minecraft:sunflower_plains",
        131 | 162 => "minecraft:windswept_gravelly_hills",
        132 => "minecraft:flower_forest",
        140 => "minecraft:ice_spikes",
        155 | 156 => "minecraft:old_growth_birch_forest",
        160 | 161 => "minecraft:old_growth_spruce_taiga",
        163 | 164 => "minecraft:windswept_savanna",
        165 => "minecraft:eroded_badlands",
        168 | 169 => "minecraft:bamboo_jungle",
        170 => "minecraft:soul_sand_valley",
        171 => "minecraft:crimson_forest",
        172 => "minecraft:warped_forest",
        173 => "minecraft:basalt_deltas",
        174 => "minecraft:dripstone_caves",
        175 => "minecraft:lush_caves",
        _ => "minecraft:plains",
    }
}

/// Build a section palette from numeric `(id, data)` pairs: returns the
/// palette (deduplicated by resolved name) and per-block palette indices.
/// `pairs` yields the 4096 blocks in the caller's index order.
pub fn palette_from_ids(pairs: impl Iterator<Item = (u16, u8)>, name: fn(u16, u8) -> &'static str) -> (Vec<crate::BlockState>, Vec<u16>) {
    use std::collections::HashMap;
    let mut by_pair: HashMap<(u16, u8), u16> = HashMap::new();
    let mut by_name: HashMap<&'static str, u16> = HashMap::new();
    let mut palette = vec![crate::BlockState::air()];
    by_name.insert("minecraft:air", 0);
    let mut indices = Vec::with_capacity(4096);
    for pair in pairs {
        let i = *by_pair.entry(pair).or_insert_with(|| {
            let n = name(pair.0, pair.1);
            *by_name.entry(n).or_insert_with(|| {
                palette.push(crate::BlockState { name: n.to_string(), properties: Vec::new() });
                (palette.len() - 1) as u16
            })
        });
        indices.push(i);
    }
    (palette, indices)
}

/// Read nibble `i` of a 4-bit array (low nibble first).
#[inline]
pub fn nibble(arr: &[u8], i: usize) -> u8 {
    arr.get(i >> 1).map_or(0, |b| (b >> ((i & 1) * 4)) & 0xF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_ids_resolve() {
        assert_eq!(java_block(1, 0), "minecraft:stone");
        assert_eq!(java_block(1, 3), "minecraft:diorite");
        assert_eq!(java_block(35, 14), "minecraft:red_wool");
        assert_eq!(java_block(17, 2 | 4), "minecraft:birch_log");
        assert_eq!(java_block(162, 1), "minecraft:dark_oak_log");
        assert_eq!(java_block(251, 11), "minecraft:blue_concrete");
        assert_eq!(java_block(240, 0), "minecraft:lime_glazed_terracotta");
        assert_eq!(bedrock_block(236, 4), "minecraft:yellow_concrete");
        assert_eq!(bedrock_block(85, 1), "minecraft:spruce_fence");
        assert_eq!(bedrock_block(2, 0), "minecraft:grass_block");
        assert_eq!(bedrock_block(219, 0), "minecraft:purple_glazed_terracotta");
        assert_eq!(bedrock_block(235, 0), "minecraft:black_glazed_terracotta");
        assert_eq!(biome(3), "minecraft:windswept_hills");
        assert_eq!(biome(21), "minecraft:jungle");
    }

    #[test]
    fn palette_dedupes_by_name() {
        // Water ids 8 and 9 collapse into one entry.
        let (pal, idx) = palette_from_ids([(0, 0), (8, 0), (9, 3), (1, 0)].into_iter(), java_block);
        assert_eq!(pal.len(), 3);
        assert_eq!(idx, vec![0, 1, 1, 2]);
        assert_eq!(nibble(&[0x21], 0), 1);
        assert_eq!(nibble(&[0x21], 1), 2);
    }
}
