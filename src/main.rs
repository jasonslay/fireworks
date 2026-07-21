//! Fireworks simulator.
//!
//! Realism notes:
//! - Stars are sampled on a 3D sphere and projected to 2D, which produces the
//!   dense-rimmed look of real shell breaks.
//! - Colors follow real pyrotechnic emitters (strontium red, barium green,
//!   copper blue, sodium gold...) and evolve white-hot -> color -> ember.
//! - HDR rendering + bloom provides the glow; particles use a soft radial
//!   texture so there are no hard sprite edges.
//! - Physics: gravity, per-star aerodynamic drag, and a slowly wandering wind.

use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::{bloom::Bloom, tonemapping::Tonemapping},
    prelude::*,
    render::camera::ScalingMode,
    render::mesh::{Indices, PrimitiveTopology},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    render::view::screenshot::{save_to_disk, Screenshot},
    window::{MonitorSelection, PrimaryWindow, WindowMode},
};
use rand::{rngs::ThreadRng, thread_rng, Rng};
use std::f32::consts::TAU;
use std::time::Duration;

const GRAVITY: f32 = -240.0;
const GROUND_Y: f32 = -370.0;

fn main() {
    // Debug helper: FIREWORKS_SHOT=out.png runs windowed, saves a screenshot
    // a few seconds in, then exits.
    let shot: Option<String> = std::env::var("FIREWORKS_SHOT").ok();
    let mode = if shot.is_some() {
        WindowMode::Windowed
    } else {
        WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
    };

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Fireworks".into(),
                resolution: (1280.0, 800.0).into(),
                mode,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Launcher {
            timer: Timer::from_seconds(0.8, TimerMode::Once),
            auto: true,
        })
        .insert_resource(Wind {
            current: 0.0,
            target: 8.0,
            retarget: 5.0,
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                handle_input,
                auto_launch,
                update_wind,
                update_shells,
                update_sparks,
                update_trails,
                update_flashes,
                twinkle_stars,
                light_foreground_hills,
            ),
        );

    if let Some(path) = shot {
        app.insert_resource(ShotConfig {
            path,
            taken: false,
        })
        .add_systems(Update, debug_screenshot);
    }

    app.run();
}

#[derive(Resource)]
struct ShotConfig {
    path: String,
    taken: bool,
}

fn debug_screenshot(
    mut commands: Commands,
    time: Res<Time>,
    mut cfg: ResMut<ShotConfig>,
    mut exit: EventWriter<AppExit>,
) {
    if !cfg.taken && time.elapsed_secs() > 3.5 {
        cfg.taken = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(cfg.path.clone()));
    }
    if cfg.taken && time.elapsed_secs() > 4.5 {
        exit.write(AppExit::Success);
    }
}

// ---------------------------------------------------------------------------
// Resources & components
// ---------------------------------------------------------------------------

#[derive(Resource)]
struct ParticleTexture(Handle<Image>);

#[derive(Resource)]
struct Launcher {
    timer: Timer,
    auto: bool,
}

#[derive(Resource)]
struct Wind {
    current: f32,
    target: f32,
    retarget: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum BurstKind {
    Peony,
    Chrysanthemum,
    Willow,
    Palm,
    Ring,
    Crossette,
    Strobe,
}

#[derive(Component)]
struct Shell {
    vel: Vec2,
    fuse: f32,
    kind: BurstKind,
    palette: (Vec3, Vec3),
    tail_timer: f32,
}

#[derive(Component)]
struct Spark {
    vel: Vec2,
    life: f32,
    max_life: f32,
    /// Linear-space emitter color at full burn.
    color: Vec3,
    drag: f32,
    gravity_mul: f32,
    size: f32,
    /// Seconds between trail particles; 0 = no trail.
    trail_interval: f32,
    trail_timer: f32,
    trail_life: f32,
    /// Strobe flash rate in Hz; 0 = steady burn.
    strobe_hz: f32,
    strobe_phase: f32,
    /// Age fraction (0..1) at which this star breaks into a crossette; 0 = never.
    split_at: f32,
    seed: f32,
}

impl Default for Spark {
    fn default() -> Self {
        Spark {
            vel: Vec2::ZERO,
            life: 1.5,
            max_life: 1.5,
            color: Vec3::ONE,
            drag: 1.8,
            gravity_mul: 0.55,
            size: 3.0,
            trail_interval: 0.0,
            trail_timer: 0.0,
            trail_life: 0.35,
            strobe_hz: 0.0,
            strobe_phase: 0.0,
            split_at: 0.0,
            seed: 0.0,
        }
    }
}

#[derive(Component)]
struct TrailBit {
    vel: Vec2,
    life: f32,
    max_life: f32,
    color: Vec3,
}

#[derive(Component)]
struct Flash {
    life: f32,
    max_life: f32,
    color: Vec3,
}

#[derive(Component)]
struct Star {
    phase: f32,
    speed: f32,
    base: f32,
}

/// Invisible light source left behind by a burst; used to relight the
/// foreground hills while the stars burn.
#[derive(Component)]
struct BurstLight {
    life: f32,
    max_life: f32,
    color: Vec3,
}

#[derive(Resource)]
struct FgHillsLighting {
    mesh: Handle<Mesh>,
    /// Ridge-top vertex position per mesh column.
    columns: Vec<Vec2>,
    /// Unlit vertex color buffer (3 rows per column: top, mid, bottom).
    base_colors: Vec<[f32; 4]>,
    /// Per-column reflectance so firework light reveals surface texture.
    albedo: Vec<f32>,
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn((
        Camera2d,
        Camera {
            hdr: true,
            clear_color: ClearColorConfig::Custom(Color::linear_rgb(0.002, 0.003, 0.010)),
            ..default()
        },
        Tonemapping::TonyMcMapface,
        Bloom {
            intensity: 0.35,
            ..Bloom::NATURAL
        },
        // Fixed virtual resolution: the scene scales uniformly to fill the
        // window. The view is anchored at the bottom (viewport_origin), so
        // aspect ratios taller than 1280x800 reveal extra sky at the top
        // instead of more foreground hillside at the bottom.
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin {
                min_width: 1280.0,
                min_height: 800.0,
            },
            viewport_origin: Vec2::new(0.5, 0.0),
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_xyz(0.0, -400.0, 0.0),
    ));

    let tex = images.add(make_radial_texture(48));
    commands.insert_resource(ParticleTexture(tex.clone()));

    let mut rng = thread_rng();

    // Night-sky stars. Spread well beyond the base 1280x800 view so
    // maximized/fullscreen (and ultrawide) windows stay filled.
    for _ in 0..520 {
        let x = rng.gen_range(-1600.0..1600.0);
        let y = rng.gen_range(GROUND_Y + 30.0..1000.0);
        let size = rng.gen_range(0.8..2.4);
        let base = rng.gen_range(0.15..0.8);
        commands.spawn((
            Sprite {
                image: tex.clone(),
                color: Color::linear_rgba(base, base, base * 1.1, 1.0),
                custom_size: Some(Vec2::splat(size * 3.0)),
                ..default()
            },
            Transform::from_xyz(x, y, 0.0),
            Star {
                phase: rng.gen_range(0.0..TAU),
                speed: rng.gen_range(0.5..2.5),
                base,
            },
        ));
    }

    // Moon: crisp cratered disc plus a soft atmospheric halo behind it.
    commands.spawn((
        Sprite {
            image: tex.clone(),
            color: Color::linear_rgba(0.055, 0.060, 0.075, 1.0),
            custom_size: Some(Vec2::splat(300.0)),
            ..default()
        },
        Transform::from_xyz(470.0, 300.0, 0.9),
    ));
    let moon_tex = images.add(make_moon_texture(256));
    commands.spawn((
        Sprite {
            image: moon_tex,
            color: Color::linear_rgba(0.95, 0.93, 0.86, 1.0),
            custom_size: Some(Vec2::splat(88.0)),
            ..default()
        },
        Transform::from_xyz(470.0, 300.0, 1.0),
    ));

    // Faint sky glow hugging the horizon (valley light pollution + airglow),
    // sitting behind the mountains so the ridgeline reads as a silhouette.
    commands.spawn((
        Sprite {
            image: tex.clone(),
            color: Color::linear_rgba(0.018, 0.024, 0.052, 1.0),
            custom_size: Some(Vec2::new(4200.0, 900.0)),
            ..default()
        },
        Transform::from_xyz(0.0, GROUND_Y + 60.0, 1.8),
    ));

    // The Front Range as seen looking west from Loveland, CO. Two layers:
    // the high peaks (Longs/Meeker massif to the southwest, Mummy Range to
    // the north) with faint moonlit snowfields, and the dark hogback
    // foothills in front. Both sit behind the particles (low z), so the
    // fireworks always display in front of the mountains.
    let far = ridge_mesh_from_profile(
        &mut rng,
        FRONT_RANGE_PROFILE,
        false,
        9.0,
        0.85,
        Vec3::new(0.0006, 0.0008, 0.0022),
        Vec3::new(0.0018, 0.0024, 0.0058),
        Some(Snow {
            threshold: 230.0,
            range: 70.0,
            color: Vec3::new(0.0085, 0.0100, 0.0170),
        }),
    );
    commands.spawn((
        Mesh2d(meshes.add(far)),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(Color::WHITE))),
        Transform::from_xyz(0.0, 0.0, 2.0),
    ));

    let foothills = hogback_profile(&mut rng);
    let near = ridge_mesh_from_profile(
        &mut rng,
        &foothills,
        true,
        3.0,
        1.0,
        Vec3::new(0.00015, 0.0002, 0.0006),
        Vec3::new(0.0005, 0.0007, 0.0018),
        None,
    );
    commands.spawn((
        Mesh2d(meshes.add(near)),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(Color::WHITE))),
        Transform::from_xyz(0.0, 0.0, 2.2),
    ));

    // Foreground hills closest to the viewer, drawn in FRONT of the
    // fireworks (z above the particles). Shells launch from the valley
    // floor behind them, so rising tails emerge from behind the ridgeline
    // and falling embers sink out of sight behind it.
    let foreground = foreground_hills_profile(&mut rng);
    let mut front = ridge_mesh_from_profile(
        &mut rng,
        &foreground,
        true,
        2.5,
        1.0,
        Vec3::new(0.00005, 0.00006, 0.00014),
        Vec3::new(0.00022, 0.0003, 0.0007),
        None,
    );

    // Remember the foreground mesh geometry so bursts can relight it.
    let mut columns = Vec::new();
    let mut base_colors = Vec::new();
    if let (
        Some(bevy::render::mesh::VertexAttributeValues::Float32x3(pos)),
        Some(bevy::render::mesh::VertexAttributeValues::Float32x4(col)),
    ) = (
        front.attribute(Mesh::ATTRIBUTE_POSITION),
        front.attribute(Mesh::ATTRIBUTE_COLOR),
    ) {
        for chunk in pos.chunks(3) {
            columns.push(Vec2::new(chunk[0][0], chunk[0][1]));
        }
        base_colors = col.clone();
    }

    // Mottled per-column albedo (brush, rock, grass patches). Baked into the
    // unlit colors and reused by the burst lighting, so firework light
    // reveals the surface texture instead of a uniform wash.
    let mut albedo = Vec::with_capacity(columns.len());
    for c in &columns {
        let a = 0.55
            + 0.9 * fbm(c.x * 0.012 + 3.7, 0.0)
            + 0.35 * value_noise(c.x * 0.07, 5.0);
        albedo.push(a.clamp(0.35, 1.7));
    }
    for (i, a) in albedo.iter().enumerate() {
        for row in 0..2 {
            let c = &mut base_colors[i * 3 + row];
            c[0] *= a;
            c[1] *= a;
            c[2] *= a;
        }
    }
    front.insert_attribute(Mesh::ATTRIBUTE_COLOR, base_colors.clone());

    let front_handle = meshes.add(front);
    commands.insert_resource(FgHillsLighting {
        mesh: front_handle.clone(),
        columns,
        base_colors,
        albedo,
    });

    commands.spawn((
        Mesh2d(front_handle),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(Color::WHITE))),
        Transform::from_xyz(0.0, 0.0, 8.0),
    ));

    info!("Controls: click = launch at point, Space = finale salvo, A = toggle auto-launch, F11 = fullscreen, Esc = quit");
}

/// Skyline of the Front Range looking west from Loveland, CO, as
/// (x, height-above-GROUND_Y) control points. Screen left = south.
/// Landmarks, south to north: foothills toward Boulder, Twin Sisters, the
/// Mount Meeker / Longs Peak massif (blocky flat top with the notch between
/// the two summits), the lower Estes Park skyline, then the Mummy Range
/// (Stormy Peaks / Signal Mountain) tapering into the plains far north.
const FRONT_RANGE_PROFILE: &[(f32, f32)] = &[
    (-2400.0, 95.0),
    (-1800.0, 120.0),
    (-1300.0, 150.0),
    (-1000.0, 135.0),
    (-760.0, 155.0),
    (-660.0, 195.0), // Twin Sisters
    (-615.0, 205.0),
    (-585.0, 185.0),
    (-540.0, 150.0), // saddle
    (-475.0, 175.0),
    (-430.0, 255.0), // rising to Meeker
    (-395.0, 301.0), // Mount Meeker summit
    (-360.0, 282.0),
    (-345.0, 286.0), // the notch (Chasm Lake cirque)
    (-330.0, 278.0),
    (-300.0, 322.0), // Longs Peak summit
    (-255.0, 318.0), // the flat "beaver" top
    (-225.0, 312.0),
    (-200.0, 270.0), // steep north face
    (-160.0, 215.0),
    (-120.0, 185.0),
    (-60.0, 160.0), // Estes Park lowlands
    (0.0, 150.0),
    (60.0, 165.0),
    (130.0, 190.0),
    (200.0, 225.0),
    (270.0, 255.0),
    (330.0, 268.0), // Mummy Range, Hagues Peak
    (380.0, 262.0),
    (430.0, 240.0),
    (500.0, 215.0),
    (560.0, 200.0), // Signal Mountain
    (640.0, 185.0),
    (800.0, 160.0),
    (1000.0, 140.0),
    (1300.0, 120.0),
    (1700.0, 105.0),
    (2400.0, 90.0),
];

struct Snow {
    /// Height above GROUND_Y where snowfields begin.
    threshold: f32,
    /// Height span over which snow blends in fully.
    range: f32,
    /// Moonlit snow color (linear).
    color: Vec3,
}

/// Rounded hogback foothills in front of the high peaks: low, gentle,
/// randomly generated each run. Kept well below the high-peak skyline so
/// they never hide Longs Peak or the Mummy Range.
fn hogback_profile(rng: &mut ThreadRng) -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    let mut x = -2400.0;
    let mut h: f32 = 10.0;
    while x < 2400.0 {
        // Random walk keeps neighboring hills related instead of scattered.
        h = (h + rng.gen_range(-28.0..28.0)).clamp(-15.0, 55.0);
        pts.push((x, h));
        x += rng.gen_range(200.0..380.0);
    }
    pts
}

/// Foreground hills closest to the viewer. Their ridgeline stays at least
/// slightly above GROUND_Y everywhere so the spark-despawn line at the
/// valley floor is always hidden behind them.
fn foreground_hills_profile(rng: &mut ThreadRng) -> Vec<(f32, f32)> {
    let mut pts = Vec::new();
    let mut x = -2400.0;
    let mut h: f32 = 15.0;
    while x < 2400.0 {
        h = (h + rng.gen_range(-13.0..13.0)).clamp(5.0, 34.0);
        pts.push((x, h));
        x += rng.gen_range(60.0..130.0);
    }
    pts
}

/// Mountain silhouette mesh from a skyline profile, filled down past the
/// bottom of the screen. Control points are smoothed with cosine
/// interpolation and roughened with `detail` jitter. Three vertex rows:
/// ridge top, a "snowline" ~110 units below it, and the screen bottom, so
/// optional snow tinting stays confined to a summit band instead of
/// washing down the whole face.
fn ridge_mesh_from_profile(
    rng: &mut ThreadRng,
    profile: &[(f32, f32)],
    rounded: bool,
    detail: f32,
    height_scale: f32,
    base_color: Vec3,
    rim_color: Vec3,
    snow: Option<Snow>,
) -> Mesh {
    const BOTTOM: f32 = GROUND_Y - 1500.0;
    const GRAD_BAND: f32 = 45.0;
    const N: usize = 560;

    let x_min = profile[0].0;
    let x_max = profile[profile.len() - 1].0;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(N * 3);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(N * 3);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(N * 3);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(N * 3);
    let mut indices: Vec<u32> = Vec::with_capacity((N - 1) * 12);

    // Fixed random phases for the small-scale rocky roughness.
    let p1 = rng.gen_range(0.0..TAU);
    let p2 = rng.gen_range(0.0..TAU);

    let deep_color = base_color * 0.35;

    for i in 0..N {
        let t = i as f32 / (N - 1) as f32;
        let x = x_min + t * (x_max - x_min);

        // Linear interpolation keeps rocky summits angular; cosine easing
        // gives rounded, weathered hilltops.
        let seg = profile.windows(2).find(|w| x >= w[0].0 && x <= w[1].0);
        let h = match seg {
            Some(w) => {
                let f = (x - w[0].0) / (w[1].0 - w[0].0);
                let f = if rounded {
                    0.5 - 0.5 * (f * std::f32::consts::PI).cos()
                } else {
                    f
                };
                w[0].1 + (w[1].1 - w[0].1) * f
            }
            None => profile[profile.len() - 1].1,
        };

        // Small-scale rockiness so ridgelines aren't glassy smooth.
        let rough = detail * (0.6 * (x * 0.045 + p1).sin() + 0.4 * (x * 0.13 + p2).sin())
            + rng.gen_range(-0.5..0.5) * detail * 0.06;
        let smooth_top = GROUND_Y + h * height_scale;
        let top = smooth_top + rough;
        // Mid row follows the smooth profile so the face gradient has no
        // per-column streaks from the roughness jitter.
        let mid = (smooth_top - GRAD_BAND).max(BOTTOM + 1.0);

        // Moonlit face near the ridgeline fading into darkness below;
        // high summits blend further into snow.
        let mut top_color = rim_color;
        if let Some(s) = &snow {
            let f = ((h - s.threshold) / s.range).clamp(0.0, 1.0);
            top_color = top_color.lerp(s.color, f);
        }

        positions.push([x, top, 0.0]);
        positions.push([x, mid, 0.0]);
        positions.push([x, BOTTOM, 0.0]);
        colors.push([top_color.x, top_color.y, top_color.z, 1.0]);
        colors.push([base_color.x, base_color.y, base_color.z, 1.0]);
        colors.push([deep_color.x, deep_color.y, deep_color.z, 1.0]);
        uvs.push([t, 0.0]);
        uvs.push([t, 0.5]);
        uvs.push([t, 1.0]);
        normals.push([0.0, 0.0, 1.0]);
        normals.push([0.0, 0.0, 1.0]);
        normals.push([0.0, 0.0, 1.0]);

        if i > 0 {
            let a = (i as u32 - 1) * 3;
            let b = i as u32 * 3;
            indices.extend_from_slice(&[a, a + 1, b, b, a + 1, b + 1]);
            indices.extend_from_slice(&[a + 1, a + 2, b + 1, b + 1, a + 2, b + 2]);
        }
    }

    // MAIN_WORLD kept so vertex colors can be rewritten at runtime
    // (firework light on the foreground hills).
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

fn hash2(ix: i32, iy: i32) -> f32 {
    let mut h = (ix.wrapping_mul(374_761_393) ^ iy.wrapping_mul(668_265_263)) as u32;
    h ^= h >> 13;
    h = h.wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

/// Smooth 2D value noise in [0, 1].
fn value_noise(x: f32, y: f32) -> f32 {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let a = hash2(ix, iy);
    let b = hash2(ix + 1, iy);
    let c = hash2(ix, iy + 1);
    let d = hash2(ix + 1, iy + 1);
    a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy
}

fn fbm(x: f32, y: f32) -> f32 {
    0.5 * value_noise(x, y) + 0.3 * value_noise(x * 2.1, y * 2.1) + 0.2 * value_noise(x * 4.3, y * 4.3)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Procedural full moon: anti-aliased disc with limb darkening and noise-based
/// maria (the dark basalt plains), clustered off-center like the real near side.
fn make_moon_texture(size: u32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let half = (size as f32 - 1.0) / 2.0;
    // Disc fills most of the texture, leaving margin for the soft edge.
    let radius = 0.94;

    for y in 0..size {
        for x in 0..size {
            // Normalized coords, y up.
            let px = (x as f32 - half) / half;
            let py = (half - y as f32) / half;
            let d = (px * px + py * py).sqrt() / radius;

            // Anti-aliased rim.
            let alpha = 1.0 - smoothstep(0.985, 1.015, d);
            if alpha <= 0.0 {
                data.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }

            let nz = (1.0 - (d * d).min(1.0)).sqrt();
            // Limb darkening: full moons stay bright almost to the edge.
            let mut v = 0.60 + 0.40 * nz.powf(0.55);

            // Maria: darker blotches biased toward the upper-left of the
            // disc (Imbrium/Serenitatis side), like the view from Earth.
            let n = fbm(px * 3.1 + 7.3, py * 3.1 - 2.6);
            let cluster = 1.0 - smoothstep(0.15, 0.95, ((px + 0.22).powi(2) + (py - 0.18).powi(2)).sqrt());
            let maria = smoothstep(0.45, 0.62, n) * cluster;
            v *= 1.0 - 0.45 * maria;

            // Fine grain so the surface isn't airbrushed-smooth.
            let grain = value_noise(px * 14.0 + 31.0, py * 14.0 + 17.0);
            v *= 0.96 + 0.07 * grain;

            // Slightly warm grey, converted to sRGB bytes.
            let (r, g, b) = (v, v * 0.985, v * 0.955);
            let to8 = |c: f32| (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
            data.extend_from_slice(&[to8(r), to8(g), to8(b), (alpha * 255.0) as u8]);
        }
    }

    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Soft radial gradient used by every particle: bright core, long falloff.
fn make_radial_texture(size: u32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let half = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 - half) / half;
            let dy = (y as f32 - half) / half;
            let d = (dx * dx + dy * dy).sqrt().min(1.0);
            // Two-term falloff: tight hot core plus a wide faint halo.
            let a = 0.85 * (1.0 - d).powi(3) + 0.15 * (1.0 - d).powf(1.2);
            let a8 = (a.clamp(0.0, 1.0) * 255.0) as u8;
            data.extend_from_slice(&[255, 255, 255, a8]);
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

// ---------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------

/// Real pyrotechnic emitter colors (linear RGB).
const COLORS: &[Vec3] = &[
    Vec3::new(1.00, 0.07, 0.04), // strontium red
    Vec3::new(1.00, 0.32, 0.05), // calcium orange
    Vec3::new(1.00, 0.55, 0.12), // sodium gold
    Vec3::new(0.10, 1.00, 0.18), // barium green
    Vec3::new(0.12, 0.35, 1.00), // copper blue
    Vec3::new(0.65, 0.15, 1.00), // strontium+copper purple
    Vec3::new(1.00, 0.25, 0.45), // pink
    Vec3::new(0.90, 0.93, 1.00), // magnesium silver
];

fn random_palette(rng: &mut ThreadRng) -> (Vec3, Vec3) {
    let a = COLORS[rng.gen_range(0..COLORS.len())];
    // Two-tone shells (colored pistil) are common; sometimes mono-color.
    let b = if rng.gen_bool(0.6) {
        COLORS[rng.gen_range(0..COLORS.len())]
    } else {
        a
    };
    (a, b)
}

fn random_kind(rng: &mut ThreadRng) -> BurstKind {
    match rng.gen_range(0..100) {
        0..=25 => BurstKind::Peony,
        26..=47 => BurstKind::Chrysanthemum,
        48..=59 => BurstKind::Willow,
        60..=69 => BurstKind::Palm,
        70..=79 => BurstKind::Ring,
        80..=89 => BurstKind::Crossette,
        _ => BurstKind::Strobe,
    }
}

fn launch_shell(
    commands: &mut Commands,
    tex: &Handle<Image>,
    rng: &mut ThreadRng,
    launch_x: f32,
    apex_y: f32,
) {
    let h = (apex_y - GROUND_Y).max(120.0);
    let v0 = (2.0 * -GRAVITY * h).sqrt();
    let fuse = (v0 / -GRAVITY) * rng.gen_range(0.86..0.97);
    commands.spawn((
        Sprite {
            image: tex.clone(),
            color: Color::linear_rgba(9.0, 6.5, 3.5, 1.0),
            custom_size: Some(Vec2::splat(9.0)),
            ..default()
        },
        Transform::from_xyz(launch_x, GROUND_Y, 5.0),
        Shell {
            vel: Vec2::new(rng.gen_range(-24.0..24.0), v0),
            fuse,
            kind: random_kind(rng),
            palette: random_palette(rng),
            tail_timer: 0.0,
        },
    ));
}

fn auto_launch(
    mut commands: Commands,
    time: Res<Time>,
    mut launcher: ResMut<Launcher>,
    tex: Res<ParticleTexture>,
) {
    if !launcher.auto {
        return;
    }
    launcher.timer.tick(time.delta());
    if !launcher.timer.finished() {
        return;
    }
    let mut rng = thread_rng();
    let count = if rng.gen_bool(0.18) { rng.gen_range(2..4) } else { 1 };
    for _ in 0..count {
        let x = rng.gen_range(-540.0..540.0);
        let apex = rng.gen_range(70.0..400.0);
        launch_shell(&mut commands, &tex.0, &mut rng, x, apex);
    }
    launcher
        .timer
        .set_duration(Duration::from_secs_f32(rng.gen_range(0.6..1.8)));
    launcher.timer.reset();
}

fn handle_input(
    mut commands: Commands,
    tex: Res<ParticleTexture>,
    mut launcher: ResMut<Launcher>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform)>,
    mut exit: EventWriter<AppExit>,
) {
    let mut rng = thread_rng();

    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if keys.just_pressed(KeyCode::KeyA) {
        launcher.auto = !launcher.auto;
        info!("auto-launch: {}", launcher.auto);
    }
    if keys.just_pressed(KeyCode::F11) {
        if let Ok(mut window) = windows.single_mut() {
            window.mode = match window.mode {
                WindowMode::Windowed => {
                    WindowMode::BorderlessFullscreen(MonitorSelection::Current)
                }
                _ => WindowMode::Windowed,
            };
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        // Finale salvo.
        for _ in 0..8 {
            let x = rng.gen_range(-580.0..580.0);
            let apex = rng.gen_range(40.0..420.0);
            launch_shell(&mut commands, &tex.0, &mut rng, x, apex);
        }
    }
    if mouse.just_pressed(MouseButton::Left) {
        let (Ok(window), Ok((camera, cam_tf))) = (windows.single_mut(), camera_q.single()) else {
            return;
        };
        if let Some(cursor) = window.cursor_position() {
            if let Ok(world) = camera.viewport_to_world_2d(cam_tf, cursor) {
                let x = world.x + rng.gen_range(-15.0..15.0);
                launch_shell(&mut commands, &tex.0, &mut rng, x, world.y);
            }
        }
    }
}

fn update_wind(time: Res<Time>, mut wind: ResMut<Wind>) {
    let dt = time.delta_secs();
    wind.retarget -= dt;
    if wind.retarget <= 0.0 {
        let mut rng = thread_rng();
        wind.target = rng.gen_range(-18.0..18.0);
        wind.retarget = rng.gen_range(4.0..9.0);
    }
    wind.current += (wind.target - wind.current) * (0.3 * dt).min(1.0);
}

// ---------------------------------------------------------------------------
// Shells
// ---------------------------------------------------------------------------

fn update_shells(
    mut commands: Commands,
    time: Res<Time>,
    wind: Res<Wind>,
    tex: Res<ParticleTexture>,
    mut shells: Query<(Entity, &mut Shell, &mut Transform)>,
) {
    let dt = time.delta_secs();
    let mut rng = thread_rng();

    for (entity, mut shell, mut tf) in &mut shells {
        shell.fuse -= dt;
        shell.vel.y += GRAVITY * dt;
        shell.vel *= (1.0 - 0.12 * dt).max(0.0);
        shell.vel.x += wind.current * 0.3 * dt;
        tf.translation.x += shell.vel.x * dt;
        tf.translation.y += shell.vel.y * dt;

        // Sparky propellant tail.
        shell.tail_timer -= dt;
        while shell.tail_timer <= 0.0 {
            shell.tail_timer += 0.012;
            let jitter = Vec2::new(rng.gen_range(-2.5..2.5), rng.gen_range(-2.5..2.5));
            let pos = tf.translation.truncate() + jitter;
            spawn_trail_bit(
                &mut commands,
                &tex.0,
                pos,
                -shell.vel * 0.08 + Vec2::new(rng.gen_range(-14.0..14.0), rng.gen_range(-20.0..6.0)),
                Vec3::new(1.0, 0.55, 0.15),
                rng.gen_range(0.18..0.4),
                rng.gen_range(1.6..2.6),
            );
        }

        if shell.fuse <= 0.0 {
            let pos = tf.translation.truncate();
            spawn_burst(&mut commands, &tex.0, &mut rng, pos, shell.kind, shell.palette);
            commands.entity(entity).despawn();
        }
    }
}

// ---------------------------------------------------------------------------
// Bursts
// ---------------------------------------------------------------------------

/// Uniform direction on a 3D sphere projected to the screen plane.
/// The projected length falls off toward the silhouette edge, giving the
/// characteristic dense rim of a real shell break.
fn shell_dir(rng: &mut ThreadRng) -> Vec2 {
    let w: f32 = rng.gen_range(-1.0..1.0);
    let a: f32 = rng.gen_range(0.0..TAU);
    Vec2::from_angle(a) * (1.0 - w * w).sqrt()
}

fn spawn_spark(commands: &mut Commands, tex: &Handle<Image>, pos: Vec2, spark: Spark) {
    let quad = spark.size * 4.2;
    commands.spawn((
        Sprite {
            image: tex.clone(),
            color: Color::linear_rgba(12.0, 12.0, 12.0, 1.0),
            custom_size: Some(Vec2::splat(quad)),
            ..default()
        },
        Transform::from_xyz(pos.x, pos.y, 4.0),
        spark,
    ));
}

fn spawn_flash(
    commands: &mut Commands,
    tex: &Handle<Image>,
    pos: Vec2,
    size: f32,
    life: f32,
    color: Vec3,
) {
    commands.spawn((
        Sprite {
            image: tex.clone(),
            color: Color::linear_rgba(color.x * 10.0, color.y * 10.0, color.z * 10.0, 1.0),
            custom_size: Some(Vec2::splat(size)),
            ..default()
        },
        Transform::from_xyz(pos.x, pos.y, 6.0),
        Flash {
            life,
            max_life: life,
            color,
        },
    ));
}

fn spawn_burst(
    commands: &mut Commands,
    tex: &Handle<Image>,
    rng: &mut ThreadRng,
    pos: Vec2,
    kind: BurstKind,
    pal: (Vec3, Vec3),
) {
    // The initial detonation flash that briefly lights the sky.
    let flash_color = pal.0 * 0.4 + Vec3::splat(0.6);
    spawn_flash(commands, tex, pos, rng.gen_range(220.0..340.0), 0.17, flash_color);

    // Invisible light that tints the foreground hills while the stars burn.
    commands.spawn((
        Transform::from_xyz(pos.x, pos.y, 0.0),
        BurstLight {
            life: 1.9,
            max_life: 1.9,
            color: pal.0 * 0.75 + Vec3::splat(0.25),
        },
    ));

    match kind {
        BurstKind::Peony => {
            let n = rng.gen_range(170..250);
            let speed = rng.gen_range(300.0..400.0);
            for _ in 0..n {
                let life = rng.gen_range(1.3..1.8);
                spawn_spark(
                    commands,
                    tex,
                    pos,
                    Spark {
                        vel: shell_dir(rng) * speed * rng.gen_range(0.93..1.07),
                        life,
                        max_life: life,
                        color: pal.0,
                        drag: 1.9,
                        size: rng.gen_range(2.4..3.4),
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
            // Colored pistil (inner core).
            let n_core = n / 3;
            for _ in 0..n_core {
                let life = rng.gen_range(1.0..1.4);
                spawn_spark(
                    commands,
                    tex,
                    pos,
                    Spark {
                        vel: shell_dir(rng) * speed * 0.42 * rng.gen_range(0.85..1.15),
                        life,
                        max_life: life,
                        color: pal.1,
                        drag: 1.9,
                        size: rng.gen_range(2.2..3.0),
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
        }
        BurstKind::Chrysanthemum => {
            let n = rng.gen_range(140..200);
            let speed = rng.gen_range(290.0..380.0);
            for _ in 0..n {
                let life = rng.gen_range(1.5..2.0);
                spawn_spark(
                    commands,
                    tex,
                    pos,
                    Spark {
                        vel: shell_dir(rng) * speed * rng.gen_range(0.93..1.07),
                        life,
                        max_life: life,
                        color: pal.0,
                        drag: 1.8,
                        size: rng.gen_range(2.4..3.2),
                        trail_interval: 0.024,
                        trail_life: 0.42,
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
        }
        BurstKind::Willow => {
            let gold = Vec3::new(1.0, 0.55, 0.14);
            let n = rng.gen_range(70..110);
            let speed = rng.gen_range(200.0..260.0);
            for _ in 0..n {
                let life = rng.gen_range(2.6..3.5);
                spawn_spark(
                    commands,
                    tex,
                    pos,
                    Spark {
                        vel: shell_dir(rng) * speed * rng.gen_range(0.9..1.1),
                        life,
                        max_life: life,
                        color: gold,
                        drag: 1.1,
                        gravity_mul: 0.5,
                        size: rng.gen_range(2.2..3.0),
                        trail_interval: 0.03,
                        trail_life: 0.9,
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
        }
        BurstKind::Palm => {
            let gold = Vec3::new(1.0, 0.5, 0.1);
            // A handful of thick rising "fronds" with heavy trails.
            let n = rng.gen_range(9..14);
            for _ in 0..n {
                let mut dir = shell_dir(rng);
                dir.y += 0.35; // palms bias upward
                let life = rng.gen_range(1.4..1.9);
                spawn_spark(
                    commands,
                    tex,
                    pos,
                    Spark {
                        vel: dir.normalize_or_zero() * rng.gen_range(170.0..240.0),
                        life,
                        max_life: life,
                        color: gold,
                        drag: 1.3,
                        gravity_mul: 0.6,
                        size: rng.gen_range(5.0..6.5),
                        trail_interval: 0.012,
                        trail_life: 0.6,
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
            // A small silver crackle crown at the center.
            for _ in 0..40 {
                let life = rng.gen_range(0.4..0.9);
                spawn_spark(
                    commands,
                    tex,
                    pos,
                    Spark {
                        vel: shell_dir(rng) * rng.gen_range(40.0..130.0),
                        life,
                        max_life: life,
                        color: Vec3::new(0.95, 0.9, 0.8),
                        drag: 2.4,
                        size: rng.gen_range(1.6..2.4),
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
        }
        BurstKind::Ring => {
            let n = rng.gen_range(70..110);
            let speed = rng.gen_range(300.0..360.0);
            let squash = rng.gen_range(0.35..1.0);
            let tilt = rng.gen_range(0.0..TAU);
            let rot = Vec2::from_angle(tilt);
            for i in 0..n {
                let a = i as f32 / n as f32 * TAU + rng.gen_range(-0.02..0.02);
                let dir = Vec2::new(a.cos(), a.sin() * squash);
                let life = rng.gen_range(1.2..1.5);
                spawn_spark(
                    commands,
                    tex,
                    pos,
                    Spark {
                        vel: rot.rotate(dir) * speed * rng.gen_range(0.97..1.03),
                        life,
                        max_life: life,
                        color: pal.0,
                        drag: 2.0,
                        size: rng.gen_range(2.6..3.4),
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
        }
        BurstKind::Crossette => {
            let n = rng.gen_range(18..30);
            let speed = rng.gen_range(220.0..300.0);
            for _ in 0..n {
                let life = rng.gen_range(1.6..2.1);
                spawn_spark(
                    commands,
                    tex,
                    pos,
                    Spark {
                        vel: shell_dir(rng) * speed * rng.gen_range(0.9..1.1),
                        life,
                        max_life: life,
                        color: pal.0,
                        drag: 1.5,
                        size: rng.gen_range(4.0..5.0),
                        trail_interval: 0.03,
                        trail_life: 0.3,
                        split_at: rng.gen_range(0.4..0.52),
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
        }
        BurstKind::Strobe => {
            let n = rng.gen_range(90..130);
            let speed = rng.gen_range(260.0..340.0);
            // Strobes are usually silver-white, occasionally tinted.
            let color = if rng.gen_bool(0.5) {
                Vec3::new(0.95, 0.95, 1.0)
            } else {
                pal.0 * 0.5 + Vec3::splat(0.5)
            };
            for _ in 0..n {
                let life = rng.gen_range(2.2..3.1);
                spawn_spark(
                    commands,
                    tex,
                    pos,
                    Spark {
                        vel: shell_dir(rng) * speed * rng.gen_range(0.88..1.12),
                        life,
                        max_life: life,
                        color,
                        drag: 2.2,
                        gravity_mul: 0.35,
                        size: rng.gen_range(2.4..3.2),
                        strobe_hz: rng.gen_range(8.0..13.0),
                        strobe_phase: rng.gen_range(0.0..1.0),
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sparks
// ---------------------------------------------------------------------------

fn update_sparks(
    mut commands: Commands,
    time: Res<Time>,
    wind: Res<Wind>,
    tex: Res<ParticleTexture>,
    mut sparks: Query<(Entity, &mut Spark, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let mut rng = thread_rng();

    for (entity, mut spark, mut tf, mut sprite) in &mut sparks {
        spark.life -= dt;
        if spark.life <= 0.0 || tf.translation.y < GROUND_Y {
            commands.entity(entity).despawn();
            continue;
        }

        // Physics.
        let damp = (1.0 - spark.drag * dt).max(0.0);
        spark.vel *= damp;
        let g = GRAVITY * spark.gravity_mul * dt;
        spark.vel.y += g;
        spark.vel.x += wind.current * dt;
        tf.translation.x += spark.vel.x * dt;
        tf.translation.y += spark.vel.y * dt;

        let age = 1.0 - spark.life / spark.max_life;

        // Crossette break: the star pops into a small cross of fragments.
        if spark.split_at > 0.0 && age >= spark.split_at {
            let pos = tf.translation.truncate();
            spawn_flash(&mut commands, &tex.0, pos, 60.0, 0.1, spark.color);
            let base_angle = rng.gen_range(0.0..TAU);
            for i in 0..4 {
                let a = base_angle + i as f32 * TAU / 4.0;
                let life = rng.gen_range(0.5..0.8);
                spawn_spark(
                    &mut commands,
                    &tex.0,
                    pos,
                    Spark {
                        vel: spark.vel * 0.3 + Vec2::from_angle(a) * rng.gen_range(110.0..150.0),
                        life,
                        max_life: life,
                        color: spark.color,
                        drag: 1.8,
                        size: 2.6,
                        seed: rng.gen_range(0.0..1.0),
                        ..default()
                    },
                );
            }
            commands.entity(entity).despawn();
            continue;
        }

        // Trails.
        if spark.trail_interval > 0.0 {
            spark.trail_timer -= dt;
            while spark.trail_timer <= 0.0 {
                spark.trail_timer += spark.trail_interval;
                let jitter = Vec2::new(rng.gen_range(-1.5..1.5), rng.gen_range(-1.5..1.5));
                spawn_trail_bit(
                    &mut commands,
                    &tex.0,
                    tf.translation.truncate() + jitter,
                    spark.vel * 0.06,
                    spark.color,
                    spark.trail_life * rng.gen_range(0.7..1.3),
                    spark.size * 0.7,
                );
            }
        }

        // Appearance.
        let (rgb, alpha) = spark_color(&spark, age, now);
        sprite.color = Color::linear_rgba(rgb.x, rgb.y, rgb.z, alpha);
        let scale = 0.55 + 0.45 * (1.0 - age);
        tf.scale = Vec3::splat(scale);
    }
}

/// Color evolution of a burning star: white-hot flash at ignition, steady
/// colored burn, then a dimming orange ember before extinction.
fn spark_color(spark: &Spark, age: f32, now: f32) -> (Vec3, f32) {
    let hot = (1.0 - age / 0.10).clamp(0.0, 1.0);
    let mut c = spark.color.lerp(Vec3::ONE, hot * 0.85);

    let ember_f = ((age - 0.72) / 0.28).clamp(0.0, 1.0);
    c = c.lerp(Vec3::new(1.0, 0.22, 0.04), ember_f * 0.65);

    // HDR intensity: ignition flash, then a steady decay.
    let mut i = 9.0 * (1.0 - age).powf(1.6) + 0.35;
    i *= 1.0 + hot * 5.0;

    // Burn flicker.
    let fl = (now * 37.0 + spark.seed * 100.0).sin() * (now * 23.0 + spark.seed * 57.0).sin();
    i *= 0.82 + 0.18 * fl.abs();

    if spark.strobe_hz > 0.0 {
        let phase = (now * spark.strobe_hz + spark.strobe_phase).fract();
        i *= if phase < 0.42 { 1.8 } else { 0.02 };
    }

    let alpha = 1.0 - ((age - 0.75) / 0.25).clamp(0.0, 1.0);
    (c * i, alpha.powf(0.7))
}

// ---------------------------------------------------------------------------
// Trails & flashes
// ---------------------------------------------------------------------------

fn spawn_trail_bit(
    commands: &mut Commands,
    tex: &Handle<Image>,
    pos: Vec2,
    vel: Vec2,
    color: Vec3,
    life: f32,
    size: f32,
) {
    commands.spawn((
        Sprite {
            image: tex.clone(),
            color: Color::linear_rgba(color.x * 3.0, color.y * 3.0, color.z * 3.0, 1.0),
            custom_size: Some(Vec2::splat(size * 4.0)),
            ..default()
        },
        Transform::from_xyz(pos.x, pos.y, 3.0),
        TrailBit {
            vel,
            life,
            max_life: life,
            color,
        },
    ));
}

fn update_trails(
    mut commands: Commands,
    time: Res<Time>,
    mut trails: Query<(Entity, &mut TrailBit, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (entity, mut bit, mut tf, mut sprite) in &mut trails {
        bit.life -= dt;
        if bit.life <= 0.0 || tf.translation.y < GROUND_Y {
            commands.entity(entity).despawn();
            continue;
        }
        bit.vel *= (1.0 - 1.5 * dt).max(0.0);
        bit.vel.y += GRAVITY * 0.18 * dt;
        tf.translation.x += bit.vel.x * dt;
        tf.translation.y += bit.vel.y * dt;

        let frac = bit.life / bit.max_life; // 1 -> 0
        let ember = bit.color.lerp(Vec3::new(1.0, 0.3, 0.05), 1.0 - frac);
        let i = 2.6 * frac * frac;
        sprite.color = Color::linear_rgba(ember.x * i, ember.y * i, ember.z * i, frac);
        tf.scale = Vec3::splat(0.4 + 0.6 * frac);
    }
}

fn update_flashes(
    mut commands: Commands,
    time: Res<Time>,
    mut flashes: Query<(Entity, &mut Flash, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (entity, mut flash, mut tf, mut sprite) in &mut flashes {
        flash.life -= dt;
        if flash.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let frac = flash.life / flash.max_life; // 1 -> 0
        let i = 10.0 * frac * frac;
        let c = flash.color;
        sprite.color = Color::linear_rgba(c.x * i, c.y * i, c.z * i, frac);
        tf.scale = Vec3::splat(1.0 + 0.8 * (1.0 - frac));
    }
}

/// Tints the foreground hills with the light of active bursts: each burst
/// leaves a `BurstLight` whose color is splatted onto the ridge-top vertex
/// colors with distance falloff, brightest at the ridgeline and fading
/// partway down the face.
fn light_foreground_hills(
    mut commands: Commands,
    time: Res<Time>,
    cfg: Res<FgHillsLighting>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut lights: Query<(Entity, &mut BurstLight, &Transform)>,
    mut was_lit: Local<bool>,
) {
    let dt = time.delta_secs();

    let mut active: Vec<(Vec2, Vec3)> = Vec::new();
    for (entity, mut light, tf) in &mut lights {
        light.life -= dt;
        if light.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let t = 1.0 - light.life / light.max_life;
        // Sharp flash at detonation, then decay along with the star burn.
        let env = if t < 0.08 {
            (t / 0.08) * 1.6
        } else {
            1.6 * (1.0 - (t - 0.08) / 0.92).powi(2)
        };
        active.push((tf.translation.truncate(), light.color * env));
    }

    if active.is_empty() && !*was_lit {
        return;
    }
    *was_lit = !active.is_empty();

    let Some(mesh) = meshes.get_mut(&cfg.mesh) else {
        return;
    };
    let mut colors = cfg.base_colors.clone();

    const R2: f32 = 520.0 * 520.0;
    for (i, top) in cfg.columns.iter().enumerate() {
        let mut lum = Vec3::ZERO;
        for (pos, color) in &active {
            let d2 = top.distance_squared(*pos);
            lum += *color * (R2 / (d2 + R2));
        }
        lum *= 0.045 * cfg.albedo[i];
        let base = i * 3;
        // Ridge top gets full light, the mid row a fraction, bottom stays dark.
        for (row, k) in [(0usize, 1.0f32), (1, 0.30)] {
            let c = &mut colors[base + row];
            c[0] += lum.x * k;
            c[1] += lum.y * k;
            c[2] += lum.z * k;
        }
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
}

fn twinkle_stars(time: Res<Time>, mut stars: Query<(&Star, &mut Sprite)>) {
    let t = time.elapsed_secs();
    for (star, mut sprite) in &mut stars {
        let b = star.base * (0.75 + 0.25 * (t * star.speed + star.phase).sin());
        sprite.color = Color::linear_rgba(b, b, b * 1.1, 1.0);
    }
}
