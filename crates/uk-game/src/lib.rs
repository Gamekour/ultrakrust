//! ULTRAKILL level runtime for ULTRAKRUST: ports of the progression scripts
//! (ObjectActivator, Door/DoorController, ActivateArena/ActivateNextWave,
//! Breakable, Glass, CheckPoint, DeathZone, TeleportPlayer, FinalDoor, ...),
//! enemies and combat, on top of `uk-core`'s movement and collision.

pub mod anim;
pub mod bot;
pub mod enemy;
pub mod game;
pub mod nav;
pub mod parity;
pub mod scripts;
pub mod ugui;

pub use game::{Game, GameEvent};
