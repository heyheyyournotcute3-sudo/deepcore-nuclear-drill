//! Walks the tech tree and prints the raw ore needed per Era, so pacing can be checked on paper.
//!
//! Usage: `cargo run -p deepcore_shared --bin balance_report [content_dir]`

use std::collections::HashMap;

use deepcore_shared::content::Content;
use deepcore_shared::content::ids::{ItemId, TechId};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| deepcore_shared::DEFAULT_CONTENT_DIR.to_string());
    let content = match Content::load_from_dir(&dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("content error: {e}");
            std::process::exit(1);
        }
    };

    println!("Deepcore balance report — content hash {:016x}", content.hash);
    println!(
        "blocks: {}  items: {}  recipes: {}  machines: {}  techs: {}  layers: {}",
        content.blocks.len(),
        content.items.len(),
        content.recipes.len(),
        content.machines.len(),
        content.techs.len(),
        content.layers.len()
    );
    println!();

    // Raw ores are the items produced by ore blocks.
    let mut raw_items: Vec<ItemId> = Vec::new();
    for block in &content.blocks {
        for (item, _) in &block.drops {
            if !raw_items.contains(item) {
                raw_items.push(*item);
            }
        }
    }

    for era in 0..=5u8 {
        let nodes: Vec<TechId> = content
            .techs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.def.era == era)
            .map(|(i, _)| TechId(i as u16))
            .collect();
        if nodes.is_empty() {
            continue;
        }
        println!("=== Era {era} ===");
        let mut totals: HashMap<ItemId, u64> = HashMap::new();
        for node in &nodes {
            let mut need: HashMap<ItemId, f64> = HashMap::new();
            for (item, count) in &content.tech(*node).cost {
                *need.entry(*item).or_insert(0.0) += f64::from(*count);
            }
            expand(&content, &mut need, 0);
            println!("  {:<26} {:>6.1} raw items", content.tech(*node).def.name, sum(&need));
            for (item, count) in &need {
                if raw_items.contains(item) {
                    *totals.entry(*item).or_insert(0) += count.ceil() as u64;
                }
            }
        }
        println!("  raw ore for the era:");
        let mut rows: Vec<(&str, u64)> = totals
            .iter()
            .map(|(i, c)| (content.item_name(*i), *c))
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        for (name, count) in rows {
            println!("    {name:<20} {count}");
        }
        println!();
    }

    // Machine build costs, which are the real material sink.
    println!("=== Machine build costs ===");
    for machine in &content.machines {
        let mut need: HashMap<ItemId, f64> = HashMap::new();
        for (item, count) in &machine.build_cost {
            *need.entry(*item).or_insert(0.0) += f64::from(*count);
        }
        expand(&content, &mut need, 0);
        println!(
            "  {:<24} {:>7.1} raw items   ({} direct)",
            machine.def.name,
            sum(&need),
            machine.build_cost.len()
        );
    }

    println!();
    println!("=== Ore use check ===");
    let mut dead = 0;
    for block in &content.blocks {
        for (item, _) in &block.drops {
            if !content.item_has_use(*item) {
                println!("  DEAD END: {} drops {}", block.def.id, content.item_name(*item));
                dead += 1;
            }
        }
    }
    if dead == 0 {
        println!("  every dropped item has a use");
    }

    println!();
    println!("=== Nuclear chain ===");
    for id in [
        "crush_uranium",
        "leach_uranium",
        "precipitate_yellowcake",
        "convert_uf6",
        "enrich_uranium",
        "fabricate_fuel_rod",
        "fabricate_core_cell",
        "build_nuclear_drill",
    ] {
        match content.recipe_id(id) {
            Some(recipe) => {
                let def = content.recipe(recipe);
                let ins: Vec<String> = def
                    .inputs
                    .iter()
                    .map(|(i, c)| format!("{c}x {}", content.item_name(*i)))
                    .collect();
                let outs: Vec<String> = def
                    .outputs
                    .iter()
                    .map(|(i, c)| format!("{c}x {}", content.item_name(*i)))
                    .collect();
                println!(
                    "  {:<24} {:>5.0}s  {} -> {}",
                    def.def.id,
                    def.def.time,
                    ins.join(" + "),
                    outs.join(" + ")
                );
            }
            None => println!("  MISSING RECIPE {id}"),
        }
    }
}

fn sum(map: &HashMap<ItemId, f64>) -> f64 {
    map.values().sum()
}

/// Expands items into their cheapest recipe's inputs, recursively.
fn expand(content: &Content, need: &mut HashMap<ItemId, f64>, depth: u32) {
    if depth > 12 {
        return;
    }
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 12 {
        changed = false;
        rounds += 1;
        let snapshot: Vec<(ItemId, f64)> = need.iter().map(|(i, c)| (*i, *c)).collect();
        for (item, count) in snapshot {
            if count <= 0.0 {
                continue;
            }
            let Some(recipe) = cheapest_recipe_for(content, item) else {
                continue;
            };
            let def = content.recipe(recipe);
            let produced: f64 = def
                .outputs
                .iter()
                .filter(|(i, _)| *i == item)
                .map(|(_, c)| f64::from(*c))
                .sum();
            if produced <= 0.0 {
                continue;
            }
            let runs = count / produced;
            *need.get_mut(&item).expect("present") -= count;
            for (input, input_count) in &def.inputs {
                *need.entry(*input).or_insert(0.0) += f64::from(*input_count) * runs;
            }
            changed = true;
        }
        need.retain(|_, v| *v > 0.001);
    }
}

/// The recipe that produces an item with the fewest inputs.
fn cheapest_recipe_for(content: &Content, item: ItemId) -> Option<deepcore_shared::content::ids::RecipeId> {
    content
        .recipes
        .iter()
        .enumerate()
        .filter(|(_, r)| r.outputs.iter().any(|(i, _)| *i == item))
        .min_by_key(|(_, r)| r.inputs.len())
        .map(|(i, _)| deepcore_shared::content::ids::RecipeId(i as u16))
}
