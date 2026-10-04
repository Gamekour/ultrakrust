//! Engine-agnostic core of ULTRAKRUST: a clean-room Rust reimplementation of
//! ULTRAKILL's player movement, using values read from the user's own install.

pub mod camera;
pub mod collide;
pub mod consts;
pub mod player;
pub mod revolver;
pub mod umath;

#[cfg(test)]
mod tests;
