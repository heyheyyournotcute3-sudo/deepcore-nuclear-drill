//! Player inventories: stacks, slots, and the transfer rules used by the server.

use serde::{Deserialize, Serialize};

use crate::content::Content;
use crate::content::ids::ItemId;

/// A stack of items. An empty stack has `count == 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStack {
    /// Item type.
    pub item: ItemId,
    /// Number of items.
    pub count: u16,
}

impl ItemStack {
    /// A stack of `count` items.
    pub fn new(item: ItemId, count: u16) -> Self {
        Self { item, count }
    }

    /// An empty stack.
    pub fn empty() -> Self {
        Self { item: ItemId(0), count: 0 }
    }

    /// Whether the stack holds nothing.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// A slot that may hold a stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    /// Contents, or `None` if empty.
    pub stack: Option<ItemStack>,
}

impl Default for Slot {
    fn default() -> Self {
        Self { stack: None }
    }
}

impl Slot {
    /// An empty slot.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A slot holding a stack.
    pub fn with(item: ItemId, count: u16) -> Self {
        Self { stack: Some(ItemStack::new(item, count)) }
    }

    /// Number of items in the slot.
    pub fn count(&self) -> u16 {
        self.stack.map_or(0, |s| s.count)
    }

    /// The item in the slot, if any.
    pub fn item(&self) -> Option<ItemId> {
        self.stack.map(|s| s.item)
    }
}

/// Number of hotbar slots.
pub const HOTBAR_SLOTS: usize = 9;
/// Number of backpack slots.
pub const BACKPACK_SLOTS: usize = 36;
/// Total player slots.
pub const PLAYER_SLOTS: usize = HOTBAR_SLOTS + BACKPACK_SLOTS;

/// The player's item storage: a hotbar followed by a backpack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    /// All slots, hotbar first.
    pub slots: Vec<Slot>,
    /// Index of the selected hotbar slot.
    pub selected: u8,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            slots: vec![Slot::empty(); PLAYER_SLOTS],
            selected: 0,
        }
    }
}

/// Errors returned by inventory operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryError {
    /// The slot index is out of range.
    BadSlot,
    /// The target slot holds a different item type.
    WrongItem,
    /// The target stack is already full.
    Full,
    /// The source slot is empty.
    Empty,
}

/// Where an item ended up after an insert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertResult {
    /// How many items were stored.
    pub stored: u16,
    /// How many items did not fit.
    pub leftover: u16,
}

impl Inventory {
    /// A new empty inventory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Maximum stack size of an item, from content.
    pub fn max_stack(content: &Content, item: ItemId) -> u16 {
        content.item(item).def.stack.max(1)
    }

    /// Inserts items, filling existing stacks first and then empty slots (hotbar before backpack).
    pub fn insert(&mut self, content: &Content, item: ItemId, mut count: u16) -> InsertResult {
        let max = Self::max_stack(content, item);
        for slot in &mut self.slots {
            if count == 0 {
                break;
            }
            if let Some(stack) = &mut slot.stack {
                if stack.item == item && stack.count < max {
                    let room = max - stack.count;
                    let take = room.min(count);
                    stack.count += take;
                    count -= take;
                }
            }
        }
        for slot in &mut self.slots {
            if count == 0 {
                break;
            }
            if slot.stack.is_none() {
                let take = max.min(count);
                slot.stack = Some(ItemStack::new(item, take));
                count -= take;
            }
        }
        InsertResult { stored: 0, leftover: count }
    }

    /// Total number of an item held.
    pub fn count_of(&self, item: ItemId) -> u32 {
        self.slots
            .iter()
            .filter_map(|s| s.stack)
            .filter(|s| s.item == item)
            .map(|s| u32::from(s.count))
            .sum()
    }

    /// Removes up to `count` of an item, returning how many were removed.
    pub fn remove(&mut self, item: ItemId, count: u16) -> u16 {
        let mut remaining = count;
        for slot in &mut self.slots {
            if remaining == 0 {
                break;
            }
            if let Some(stack) = &mut slot.stack {
                if stack.item == item {
                    let take = stack.count.min(remaining);
                    stack.count -= take;
                    remaining -= take;
                    if stack.count == 0 {
                        slot.stack = None;
                    }
                }
            }
        }
        count - remaining
    }

    /// Removes items from a specific slot.
    pub fn take_from_slot(&mut self, index: usize, count: u16) -> Result<ItemStack, InventoryError> {
        let slot = self.slots.get_mut(index).ok_or(InventoryError::BadSlot)?;
        let Some(stack) = slot.stack else {
            return Err(InventoryError::Empty);
        };
        let take = stack.count.min(count);
        let item = stack.item;
        if take == stack.count {
            slot.stack = None;
        } else {
            if let Some(s) = &mut slot.stack {
                s.count -= take;
            }
        }
        Ok(ItemStack::new(item, take))
    }

    /// Moves items between two slots, merging when the item types match.
    pub fn move_between(
        &mut self,
        content: &Content,
        from: usize,
        to: usize,
        count: u16,
    ) -> Result<u16, InventoryError> {
        if from == to {
            return Ok(0);
        }
        let max = {
            let source = self.slots.get(from).ok_or(InventoryError::BadSlot)?;
            let stack = source.stack.ok_or(InventoryError::Empty)?;
            Self::max_stack(content, stack.item)
        };
        let taken = self.take_from_slot(from, count)?;
        let target = self.slots.get_mut(to).ok_or(InventoryError::BadSlot)?;
        match &mut target.stack {
            None => {
                target.stack = Some(taken);
                Ok(taken.count)
            }
            Some(stack) => {
                if stack.item != taken.item {
                    // Put it back and report the conflict.
                    self.slots[from].stack = Some(taken);
                    return Err(InventoryError::WrongItem);
                }
                let room = max.saturating_sub(stack.count);
                let moved = room.min(taken.count);
                stack.count += moved;
                if moved < taken.count {
                    let rest = ItemStack::new(taken.item, taken.count - moved);
                    match &mut self.slots[from].stack {
                        Some(existing) if existing.item == rest.item => {
                            existing.count += rest.count;
                        }
                        slot => *slot = Some(rest),
                    }
                }
                Ok(moved)
            }
        }
    }

    /// Swaps two slots.
    pub fn swap(&mut self, a: usize, b: usize) -> Result<(), InventoryError> {
        if a >= self.slots.len() || b >= self.slots.len() {
            return Err(InventoryError::BadSlot);
        }
        self.slots.swap(a, b);
        Ok(())
    }

    /// Quick-moves a slot: hotbar → backpack or backpack → hotbar.
    pub fn quick_move(&mut self, content: &Content, index: usize) -> Result<u16, InventoryError> {
        let from_hotbar = index < HOTBAR_SLOTS;
        let range = if from_hotbar {
            HOTBAR_SLOTS..PLAYER_SLOTS
        } else {
            0..HOTBAR_SLOTS
        };
        let stack = self.slots.get(index).ok_or(InventoryError::BadSlot)?.stack;
        let Some(stack) = stack else {
            return Err(InventoryError::Empty);
        };
        let max = Self::max_stack(content, stack.item);
        let mut remaining = stack.count;
        // Fill matching stacks first.
        for i in range.clone() {
            if remaining == 0 {
                break;
            }
            if let Some(target) = &mut self.slots[i].stack {
                if target.item == stack.item && target.count < max {
                    let take = (max - target.count).min(remaining);
                    target.count += take;
                    remaining -= take;
                }
            }
        }
        for i in range {
            if remaining == 0 {
                break;
            }
            if self.slots[i].stack.is_none() {
                let take = max.min(remaining);
                self.slots[i].stack = Some(ItemStack::new(stack.item, take));
                remaining -= take;
            }
        }
        let moved = stack.count - remaining;
        if moved > 0 {
            let slot = &mut self.slots[index];
            match &mut slot.stack {
                Some(s) => {
                    s.count -= moved;
                    if s.count == 0 {
                        slot.stack = None;
                    }
                }
                None => {}
            }
        }
        Ok(moved)
    }

    /// The stack in the selected hotbar slot.
    pub fn selected_stack(&self) -> Option<ItemStack> {
        self.slots
            .get(usize::from(self.selected))
            .and_then(|s| s.stack)
    }

    /// Sets the selected hotbar slot.
    pub fn select(&mut self, index: u8) {
        if usize::from(index) < HOTBAR_SLOTS {
            self.selected = index;
        }
    }

    /// Whether the inventory has room for `count` of an item.
    pub fn has_room_for(&self, content: &Content, item: ItemId, count: u16) -> bool {
        let max = Self::max_stack(content, item);
        let mut room = 0u32;
        for slot in &self.slots {
            match &slot.stack {
                None => room += u32::from(max),
                Some(s) if s.item == item => room += u32::from(max - s.count),
                _ => {}
            }
            if room >= u32::from(count) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn content() -> Content {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/data");
        Content::load_from_dir(dir).expect("content loads")
    }

    #[test]
    fn insert_stacks_up_to_max() {
        let c = content();
        let coal = c.item_id("coal").expect("coal");
        let mut inv = Inventory::new();
        let r = inv.insert(&c, coal, 100);
        assert_eq!(r.leftover, 0);
        assert_eq!(inv.count_of(coal), 100);
        // 100 coal at 64 per stack = 2 slots.
        assert_eq!(inv.slots.iter().filter(|s| s.count() > 0).count(), 2);
    }

    #[test]
    fn insert_overflow_is_reported() {
        let c = content();
        let coal = c.item_id("coal").expect("coal");
        let mut inv = Inventory::new();
        let r = inv.insert(&c, coal, u16::MAX);
        assert!(r.leftover > 0, "should not fit 65535 coal");
        assert_eq!(inv.count_of(coal) + u32::from(r.leftover), u32::from(u16::MAX));
    }

    #[test]
    fn move_between_merges_and_splits() {
        let c = content();
        let coal = c.item_id("coal").expect("coal");
        let mut inv = Inventory::new();
        inv.slots[0] = Slot::with(coal, 64);
        inv.slots[1] = Slot::with(coal, 10);
        let moved = inv.move_between(&c, 0, 1, 64).expect("moves");
        assert_eq!(moved, 54);
        assert_eq!(inv.slots[1].count(), 64);
        assert_eq!(inv.slots[0].count(), 10);
    }

    #[test]
    fn quick_move_moves_between_areas() {
        let c = content();
        let coal = c.item_id("coal").expect("coal");
        let mut inv = Inventory::new();
        inv.slots[0] = Slot::with(coal, 30);
        let moved = inv.quick_move(&c, 0).expect("moves");
        assert_eq!(moved, 30);
        assert_eq!(inv.slots[0].count(), 0);
        assert_eq!(inv.slots[HOTBAR_SLOTS].count(), 30);
    }

    proptest! {
        #[test]
        fn insert_then_remove_round_trips(count in 1u16..600) {
            let c = content();
            let coal = c.item_id("coal").expect("coal");
            let mut inv = Inventory::new();
            let r = inv.insert(&c, coal, count);
            let stored = u32::from(count) - u32::from(r.leftover);
            prop_assert_eq!(inv.count_of(coal), stored);
            let removed = inv.remove(coal, count);
            prop_assert_eq!(u32::from(removed), stored);
            prop_assert_eq!(inv.count_of(coal), 0);
        }

        #[test]
        fn total_items_are_conserved_by_moves(from in 0usize..9, to in 9usize..45, count in 1u16..64) {
            let c = content();
            let coal = c.item_id("coal").expect("coal");
            let mut inv = Inventory::new();
            inv.slots[from] = Slot::with(coal, 64);
            let before = inv.count_of(coal);
            let _ = inv.move_between(&c, from, to, count);
            prop_assert_eq!(inv.count_of(coal), before);
        }
    }
}
