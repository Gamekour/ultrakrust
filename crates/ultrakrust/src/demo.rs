//! `--demo <dir>`: a scripted run of 0-1 driven by the autopilot (no input
//! injection): land, punch through the planks, take the revolver and clear the
//! Gun Room, then fight the Malicious Face and drop into the final pit. Saves a
//! screenshot at each milestone, then exits. Also used to record the showcase.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use std::path::PathBuf;
use uk_game::bot::{Bot, BotFrame, Waypoint};
use uk_game::Game;

#[derive(Clone, Debug)]
pub enum Step {
    Wait(f32),
    /// Look (yaw, pitch) for a moment.
    Look(f32, f32, f32),
    Punch,
    Shot(&'static str),
    /// Move the player (feet position), face yaw.
    Teleport(Vec3, f32),
    /// Activate a node by full path (stands in for progression we skip).
    Activate(&'static str),
    /// Run the bot along waypoints until done or timeout; fights whatever spawns.
    Route(Vec<Waypoint>, f32),
    /// Let the bot fight until no active enemies remain (or timeout).
    Fight(f32),
    Exit,
}

#[derive(Resource)]
pub struct Demo {
    pub dir: PathBuf,
    pub steps: Vec<Step>,
    pub idx: usize,
    pub t: f32,
    pub bot: Option<Bot>,
    pub look: Option<(f32, f32)>,
    pub seen_enemy: bool,
}

fn wp(x: f32, y: f32, z: f32, label: &'static str, r: f32) -> Waypoint {
    Waypoint { pos: Vec3::new(x, y, z), label, radius: r }
}

pub fn script_0_1() -> Vec<Step> {
    use Step::*;
    vec![
        Wait(4.0),
        Shot("01_landed"),
        Route(vec![wp(0.0, -0.5, -255.5, "up to the planks", 0.7)], 3.0),
        Look(0.0, -20.0, 0.4),
        Punch,
        Wait(0.6),
        Look(0.0, -35.0, 0.3),
        Punch,
        Wait(0.6),
        Look(0.0, 5.0, 0.3),
        Punch,
        Wait(0.8),
        Shot("02_planks_broken"),
        Route(vec![wp(0.0, -0.5, -262.0, "through planks", 2.0), wp(0.0, 0.0, -277.0, "corridor", 1.5)], 8.0),
        Shot("03_corridor"),
        Teleport(Vec3::new(40.25, -0.5, -380.0), 180.0),
        Route(vec![wp(40.0, 1.5, -393.0, "revolver", 1.5)], 6.0),
        Wait(2.5),
        Shot("04_revolver_acquired"),
        Route(vec![wp(40.0, 0.0, -386.0, "gun room centre", 2.0)], 4.0),
        Wait(9.0),
        Route(vec![wp(40.0, 0.0, -389.0, "gun room", 1.5)], 1.5),
        Fight(5.0),
        Shot("05_gun_room_fight"),
        Fight(40.0),
        Shot("06_gun_room_cleared"),
        Activate("13 - Malicious Face Arena"),
        Activate("12B - Pre-Boss Checkpoint"),
        Teleport(Vec3::new(202.0, 54.5, -425.0), 180.0),
        Route(vec![wp(202.0, 54.5, -416.0, "into the arena", 1.5)], 5.0),
        Fight(4.0),
        Shot("07_malicious_face"),
        Fight(60.0),
        Wait(3.0),
        Shot("08_boss_dead"),
        Teleport(Vec3::new(202.0, 54.5, -402.0), 180.0),
        Wait(2.5),
        Shot("09_final_door"),
        Teleport(Vec3::new(202.0, -10.0, -354.0), 180.0),
        Wait(2.0),
        Shot("10_level_complete"),
        Wait(1.0),
        Exit,
    ]
}

impl Demo {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, steps: script_0_1(), idx: 0, t: 0.0, bot: None, look: None, seen_enemy: false }
    }

    /// Advances the script. Returns bot control for this frame, if any.
    pub fn tick(&mut self, game: &mut Game, dt: f32, commands: &mut Commands, exit: &mut MessageWriter<AppExit>) -> Option<BotFrame> {
        let Some(step) = self.steps.get(self.idx).cloned() else { return None };
        self.t += dt;
        let next = |d: &mut Demo| {
            d.idx += 1;
            d.t = 0.0;
            d.bot = None;
            d.seen_enemy = false;
        };
        match step {
            Step::Wait(s) => {
                if self.t >= s {
                    next(self);
                }
                None
            }
            Step::Look(yaw_off, pitch, s) => {
                let yaw = game.spawn_yaw + yaw_off;
                self.look = Some((yaw, pitch));
                if self.t >= s {
                    next(self);
                }
                Some(BotFrame { yaw_deg: yaw, pitch_deg: pitch, ..default() })
            }
            Step::Punch => {
                next(self);
                let (yaw, pitch) = self.look.unwrap_or((game.s.player.yaw_deg, 0.0));
                Some(BotFrame { yaw_deg: yaw, pitch_deg: pitch, punch: true, ..default() })
            }
            Step::Shot(name) => {
                let path = self.dir.join(format!("{name}.png"));
                info!("demo shot {}", path.display());
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
                next(self);
                None
            }
            Step::Teleport(p, yaw) => {
                game.s.player.pos = p;
                game.s.player.prev_pos = p;
                game.s.player.vel = Vec3::ZERO;
                game.s.player.yaw_deg = yaw;
                self.look = Some((yaw, 0.0));
                next(self);
                Some(BotFrame { yaw_deg: yaw, ..default() })
            }
            Step::Activate(path) => {
                if let Some(n) = (0..game.def.nodes.len() as u32).find(|&n| game.def.path(n) == path) {
                    game.set_active(n, true);
                }
                next(self);
                None
            }
            Step::Route(route, timeout) => {
                let bot = self.bot.get_or_insert_with(|| Bot::new(route));
                let f = bot.think(game, dt);
                if bot.done() || self.t > timeout {
                    next(self);
                }
                Some(f)
            }
            Step::Fight(timeout) => {
                let bot = self.bot.get_or_insert_with(|| Bot::new(Vec::new()));
                let f = bot.think(game, dt);
                let any = game.s.enemies.iter().any(|e| e.alive && game.active(e.node));
                self.seen_enemy |= any;
                // ends once the enemies that showed up are all dead
                if (!any && self.seen_enemy) || self.t > timeout {
                    next(self);
                }
                Some(f)
            }
            Step::Exit => {
                exit.write(AppExit::Success);
                None
            }
        }
    }
}
