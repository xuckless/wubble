//! The start page's living bubble (`wubble://start`, the new-tab page): the design's soft-body
//! logo, its little bubbles and their pops, and the W distance field, ported from the web
//! (`core/softbody.ts`, `core/minis.ts`, `core/living.ts`, `wTexture()` in `core/gl/scene.ts`) by
//! way of the design repo's own Rust port (`slint/softbody.rs`, `slint/minis.rs`,
//! `slint/wfield.rs`). Pure: fixed steps of `dt`, pointer times passed in, no clocks.
//!
//! - `living`: the one machine the host drives (`transition(Living, Event) -> (Living, Vec<Effect>)`).
//! - `softbody`, `grab`, `minis`: what it is made of (integrators, and the pointer's hold).
//! - `frame`: what the scene's bubble and pop passes draw, as plain uniforms.
//! - `wfield`: the W's distance-field texture, computed once.
//!
//! Every tuning number is generated (`kit_gen::{scene, softbody, minis, living, start}`, read
//! from the web by `tools/fidelity/extract.mjs`); the tests replay trajectories sampled from the
//! web (`tools/fidelity/sample-start.mjs`).

pub mod frame;
pub mod grab;
pub mod living;
pub mod minis;
pub mod rand;
pub mod softbody;
pub mod wfield;

#[cfg(test)]
mod fixtures;
