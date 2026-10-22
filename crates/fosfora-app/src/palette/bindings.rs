use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteBinding {
    pub slot: String,
    pub layer: usize,
    pub parameter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteBindingSet {
    #[serde(default = "schema_v1")]
    pub schema_version: u32,
    pub preset: String,
    pub bindings: Vec<PaletteBinding>,
}

fn schema_v1() -> u32 {
    1
}

impl PaletteBindingSet {
    /// Slot-ID (not position) membership — planner p1-ui needs it for
    /// "slot still on any live palette?" checks; normal builds don't call it.
    #[cfg(test)]
    pub fn uses_slot(&self, slot: &str) -> bool {
        self.bindings.iter().any(|b| b.slot == slot)
    }
}

/// Drop a colour target whose layer was removed, and close the index gap.
pub fn drop_layer_index(set: &mut PaletteBindingSet, removed: usize) {
    set.bindings.retain_mut(|binding| {
        match crate::bindings::bus::layer_index_after_remove(binding.layer, removed) {
            Some(layer) => {
                binding.layer = layer;
                true
            }
            None => false,
        }
    });
}

/// Keep a colour target on the same layer after that layer is dragged.
pub fn remap_layer_indices(set: &mut PaletteBindingSet, from: usize, to: usize) {
    for binding in &mut set.bindings {
        binding.layer = crate::bindings::bus::layer_index_after_move(binding.layer, from, to);
    }
}

/// Write one preset's binding set beside the show pack.
pub fn save_set(pack_root: &std::path::Path, set: &PaletteBindingSet) -> std::io::Result<()> {
    let dir = pack_root.join("palette-bindings");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", set.preset));
    let text = serde_json::to_string_pretty(set)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    crate::paths::write_atomic(&path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder_keeps_the_colour_on_the_moved_layer() {
        let mut set = PaletteBindingSet {
            schema_version: 1,
            preset: "under-ice".into(),
            bindings: vec![PaletteBinding {
                slot: "color-3".into(),
                layer: 2,
                parameter: "tint".into(),
            }],
        };
        remap_layer_indices(&mut set, 2, 0);
        assert_eq!(set.bindings[0].layer, 0);
        remap_layer_indices(&mut set, 0, 2);
        assert_eq!(set.bindings[0].layer, 2);
    }

    #[test]
    fn remove_drops_the_colour_and_closes_the_gap() {
        let mut set = PaletteBindingSet {
            schema_version: 1,
            preset: "under-ice".into(),
            bindings: vec![
                PaletteBinding {
                    slot: "color-3".into(),
                    layer: 1,
                    parameter: "tint".into(),
                },
                PaletteBinding {
                    slot: "color-4".into(),
                    layer: 3,
                    parameter: "glow".into(),
                },
            ],
        };
        drop_layer_index(&mut set, 1);
        assert_eq!(set.bindings.len(), 1);
        assert_eq!(set.bindings[0].layer, 2);
        assert_eq!(set.bindings[0].parameter, "glow");
    }

    #[test]
    fn delete_bound_swatch_detected() {
        let set = PaletteBindingSet {
            schema_version: 1,
            preset: "deep-sleep".into(),
            bindings: vec![PaletteBinding {
                slot: "color-1".into(),
                layer: 0,
                parameter: "tint".into(),
            }],
        };
        assert!(set.uses_slot("color-1"));
        assert!(!set.uses_slot("color-6"));
    }
}
