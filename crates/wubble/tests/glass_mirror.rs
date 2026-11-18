//! The Slint `GlassTier` mirrors `wubble_gfx::glass::Tier`. The host is the
//! one crate that sees both, so the mirror is checked here. Exhaustive both
//! ways: adding a variant on either side fails to compile.

use wubble_gfx::glass::Tier;
use wubble_ui::GlassTier;

#[test]
fn slint_glass_tier_mirrors_rust() {
    let to_ui = |t| match t {
        Tier::Thin => GlassTier::Thin,
        Tier::Regular => GlassTier::Regular,
        Tier::Thick => GlassTier::Thick,
    };
    let from_ui = |t| match t {
        GlassTier::Thin => Tier::Thin,
        GlassTier::Regular => Tier::Regular,
        GlassTier::Thick => Tier::Thick,
    };
    for tier in Tier::ALL {
        assert_eq!(from_ui(to_ui(tier)), tier);
    }
}
