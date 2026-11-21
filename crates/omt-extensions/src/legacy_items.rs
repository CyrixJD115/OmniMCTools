//! Numeric item ids of Java worlds saved before 1.8 (items stored
//! `id:276s`): blocks 0–255 resolve through `omt_world::legacy`, items here
//! (256–453 and the music discs 2256–2267), named the flattened (1.13+) way.

/// Flattened item id of a legacy numeric id (`None` = unknown).
pub fn name(id: i64, damage: i64) -> Option<String> {
    if (0..256).contains(&id) {
        let n = omt_world::legacy::java_block(id as u16, damage.clamp(0, 15) as u8);
        return (n != "minecraft:air" || id == 0).then(|| n.to_string());
    }
    const ITEMS: [&str; 198] = [
        "iron_shovel", "iron_pickaxe", "iron_axe", "flint_and_steel", "apple", "bow", "arrow", "coal", "diamond", "iron_ingot", "gold_ingot", "iron_sword",
        "wooden_sword", "wooden_shovel", "wooden_pickaxe", "wooden_axe", "stone_sword", "stone_shovel", "stone_pickaxe", "stone_axe", "diamond_sword", "diamond_shovel",
        "diamond_pickaxe", "diamond_axe", "stick", "bowl", "mushroom_stew", "golden_sword", "golden_shovel", "golden_pickaxe", "golden_axe", "string", "feather",
        "gunpowder", "wooden_hoe", "stone_hoe", "iron_hoe", "diamond_hoe", "golden_hoe", "wheat_seeds", "wheat", "bread", "leather_helmet", "leather_chestplate",
        "leather_leggings", "leather_boots", "chainmail_helmet", "chainmail_chestplate", "chainmail_leggings", "chainmail_boots", "iron_helmet", "iron_chestplate",
        "iron_leggings", "iron_boots", "diamond_helmet", "diamond_chestplate", "diamond_leggings", "diamond_boots", "golden_helmet", "golden_chestplate",
        "golden_leggings", "golden_boots", "flint", "porkchop", "cooked_porkchop", "painting", "golden_apple", "oak_sign", "oak_door", "bucket", "water_bucket",
        "lava_bucket", "minecart", "saddle", "iron_door", "redstone", "snowball", "oak_boat", "leather", "milk_bucket", "brick", "clay_ball", "sugar_cane", "paper",
        "book", "slime_ball", "chest_minecart", "furnace_minecart", "egg", "compass", "fishing_rod", "clock", "glowstone_dust", "cod", "cooked_cod", "ink_sac",
        "bone", "sugar", "cake", "white_bed", "repeater", "cookie", "filled_map", "shears", "melon_slice", "pumpkin_seeds", "melon_seeds", "beef", "cooked_beef",
        "chicken", "cooked_chicken", "rotten_flesh", "ender_pearl", "blaze_rod", "ghast_tear", "gold_nugget", "nether_wart", "potion", "glass_bottle", "spider_eye",
        "fermented_spider_eye", "blaze_powder", "magma_cream", "brewing_stand", "cauldron", "ender_eye", "glistering_melon_slice", "pig_spawn_egg",
        "experience_bottle", "fire_charge", "writable_book", "written_book", "emerald", "item_frame", "flower_pot", "carrot", "potato", "baked_potato",
        "poisonous_potato", "map", "golden_carrot", "skeleton_skull", "carrot_on_a_stick", "nether_star", "pumpkin_pie", "firework_rocket", "firework_star",
        "enchanted_book", "comparator", "nether_brick", "quartz", "tnt_minecart", "hopper_minecart", "prismarine_shard", "prismarine_crystals", "rabbit",
        "cooked_rabbit", "rabbit_stew", "rabbit_foot", "rabbit_hide", "armor_stand", "iron_horse_armor", "golden_horse_armor", "diamond_horse_armor", "lead",
        "name_tag", "command_block_minecart", "mutton", "cooked_mutton", "white_banner", "end_crystal", "spruce_door", "birch_door", "jungle_door", "acacia_door",
        "dark_oak_door", "chorus_fruit", "popped_chorus_fruit", "beetroot", "beetroot_seeds", "beetroot_soup", "dragon_breath", "splash_potion", "spectral_arrow",
        "tipped_arrow", "lingering_potion", "shield", "elytra", "spruce_boat", "birch_boat", "jungle_boat", "acacia_boat", "dark_oak_boat", "totem_of_undying",
        "shulker_shell", "", "iron_nugget", "knowledge_book",
    ];
    const DISCS: [&str; 12] = ["13", "cat", "blocks", "chirp", "far", "mall", "mellohi", "stal", "strad", "ward", "11", "wait"];
    let n = match id {
        263 if damage == 1 => "charcoal",
        322 if damage == 1 => "enchanted_golden_apple",
        256..=453 => ITEMS[(id - 256) as usize],
        2256..=2267 => return Some(format!("minecraft:music_disc_{}", DISCS[(id - 2256) as usize])),
        _ => "",
    };
    (!n.is_empty()).then(|| format!("minecraft:{n}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn classic_ids() {
        assert_eq!(super::name(276, 0).as_deref(), Some("minecraft:diamond_sword"));
        assert_eq!(super::name(271, 0).as_deref(), Some("minecraft:wooden_axe"));
        assert_eq!(super::name(453, 0).as_deref(), Some("minecraft:knowledge_book"));
        assert_eq!(super::name(263, 1).as_deref(), Some("minecraft:charcoal"));
        assert_eq!(super::name(1, 0).as_deref(), Some("minecraft:stone"));
        assert_eq!(super::name(2258, 0).as_deref(), Some("minecraft:music_disc_blocks"));
        assert_eq!(super::name(451, 0), None);
        assert_eq!(super::name(9999, 0), None);
    }
}
