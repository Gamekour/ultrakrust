//! Values read from the user's ULTRAKILL install (build 22957324, Unity 2022.3.29f1).
//! See MODLOG.md for where each one comes from.

use bevy_math::Vec3;

/// TimeManager "Fixed Timestep" (also `NewMovement.ONE_PHYSICS_FRAME`).
pub const FIXED_DT: f32 = 0.008;
/// PhysicsManager gravity (y).
pub const GRAVITY: f32 = -40.0;

// NewMovement serialized fields (player prefab).
pub const WALK_SPEED: f32 = 750.0;
pub const JUMP_POWER: f32 = 90.0;
pub const AIR_ACCELERATION: f32 = 6000.0;
pub const WALL_JUMP_POWER: f32 = 150.0;
pub const SSJ_MAX_FRAMES: f32 = 4.0;

// Player rigidbody + capsule.
pub const MASS: f32 = 100.0;
pub const RADIUS: f32 = 0.5;
pub const STAND_HEIGHT: f32 = 3.5;
pub const SLIDE_HEIGHT: f32 = 1.25;
pub const COLLIDER_CENTER_Y: f32 = 0.25;

// GroundCheck: local (0,-1.256,0), scale (0.85,0.8,0.85), capsule r0.25 h1.3.
pub const GROUND_CHECK_POS: Vec3 = Vec3::new(0.0, -1.256, 0.0);
pub const GC_RADIUS: f32 = 0.25 * 0.85;
pub const GC_HEIGHT: f32 = 1.3 * 0.8;
// SlopeCheck: local (0,-0.1,0), capsule r0.45 h3.5 center +0.25.
pub const SLOPE_CHECK_POS: Vec3 = Vec3::new(0.0, -0.1, 0.0);
pub const SLOPE_RADIUS: f32 = 0.45;
// WallCheck: local (0,-0.1,0), scale 1.8, sphere r0.5.
pub const WALL_CHECK_POS: Vec3 = Vec3::new(0.0, -0.1, 0.0);
pub const WALL_RADIUS: f32 = 0.5 * 1.8;

/// Main Camera local position.
pub const CAMERA_POS: Vec3 = Vec3::new(0.0, 1.4, 0.0);
/// PrefsManager default "fieldOfView" (Unity vertical FOV, degrees).
pub const DEFAULT_FOV: f32 = 105.0;
