//! Simulator renderer: draws the video frames for a `MediaJob` from
//! archived telemetry. Top-down chase camera over a pre-rasterised track,
//! car + ghost, HUD, title/difference cards and a slow-motion "why it
//! works" section. Frames are RGB8 and streamed to the composer.

use rti_core::{CarState, Track, Trajectory};
use rti_sim::TrackGeom;

use crate::compare::Comparison;
use crate::font::{glyph, text_width, GLYPH_H, GLYPH_W};
use crate::job::{CameraDef, ComparisonMode, MediaJob};

pub type Rgb = [u8; 3];

pub const BG: Rgb = [14, 16, 22];
pub const ASPHALT: Rgb = [58, 66, 80];
pub const PLASTIC: Rgb = [120, 90, 160];
pub const WATER: Rgb = [40, 90, 150];
pub const SAND: Rgb = [170, 150, 100];
pub const SNOW: Rgb = [210, 215, 225];
pub const METAL: Rgb = [110, 115, 125];
pub const PENALTY: Rgb = [150, 60, 60];
pub const BUMP: Rgb = [80, 74, 70];

pub fn surface_colour(s: rti_core::Surface) -> Rgb {
    match s {
        rti_core::Surface::Asphalt => ASPHALT,
        rti_core::Surface::Dirt => DIRT,
        rti_core::Surface::Grass => GRASS,
        rti_core::Surface::Ice => ICE,
        rti_core::Surface::Bump => BUMP,
        rti_core::Surface::Plastic => PLASTIC,
        rti_core::Surface::Water => WATER,
        rti_core::Surface::Sand => SAND,
        rti_core::Surface::Snow => SNOW,
        rti_core::Surface::Metal => METAL,
        rti_core::Surface::Penalty => PENALTY,
    }
}
pub const DIRT: Rgb = [110, 86, 52];
pub const GRASS: Rgb = [46, 90, 52];
pub const ICE: Rgb = [90, 150, 190];
pub const WALL: Rgb = [200, 205, 215];
pub const NEW: Rgb = [90, 170, 255];
pub const OLD: Rgb = [255, 154, 60];
pub const TEXT: Rgb = [235, 238, 245];
pub const DIM: Rgb = [140, 148, 165];
pub const GOOD: Rgb = [79, 209, 139];
pub const BAD: Rgb = [255, 107, 107];

/// An RGB8 raster.
#[derive(Clone)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Frame {
    pub fn new(w: usize, h: usize, fill: Rgb) -> Frame {
        let mut px = Vec::with_capacity(w * h * 3);
        for _ in 0..w * h {
            px.extend_from_slice(&fill);
        }
        Frame { w, h, px }
    }

    #[inline]
    pub fn put(&mut self, x: i64, y: i64, c: Rgb) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            let i = (y as usize * self.w + x as usize) * 3;
            self.px[i..i + 3].copy_from_slice(&c);
        }
    }

    #[inline]
    pub fn blend(&mut self, x: i64, y: i64, c: Rgb, a: f32) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            let i = (y as usize * self.w + x as usize) * 3;
            for k in 0..3 {
                let v = self.px[i + k] as f32;
                self.px[i + k] = (v + (c[k] as f32 - v) * a).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    pub fn rect(&mut self, x0: i64, y0: i64, w: i64, h: i64, c: Rgb) {
        for y in y0.max(0)..(y0 + h).min(self.h as i64) {
            for x in x0.max(0)..(x0 + w).min(self.w as i64) {
                self.put(x, y, c);
            }
        }
    }

    pub fn rect_alpha(&mut self, x0: i64, y0: i64, w: i64, h: i64, c: Rgb, a: f32) {
        for y in y0.max(0)..(y0 + h).min(self.h as i64) {
            for x in x0.max(0)..(x0 + w).min(self.w as i64) {
                self.blend(x, y, c, a);
            }
        }
    }

    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, a: f32) {
        let r2 = r * r;
        for y in (cy - r).floor() as i64..=(cy + r).ceil() as i64 {
            for x in (cx - r).floor() as i64..=(cx + r).ceil() as i64 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                if dx * dx + dy * dy <= r2 {
                    if a >= 1.0 {
                        self.put(x, y, c);
                    } else {
                        self.blend(x, y, c, a);
                    }
                }
            }
        }
    }

    /// Thick line drawn as a capsule.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: Rgb, a: f32) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len2 = dx * dx + dy * dy;
        let r = width / 2.0;
        let minx = x0.min(x1) - r;
        let maxx = x0.max(x1) + r;
        let miny = y0.min(y1) - r;
        let maxy = y0.max(y1) + r;
        for y in miny.floor() as i64..=maxy.ceil() as i64 {
            for x in minx.floor() as i64..=maxx.ceil() as i64 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let t = if len2 > 0.0 {
                    ((px - x0) * dx + (py - y0) * dy) / len2
                } else {
                    0.0
                }
                .clamp(0.0, 1.0);
                let qx = x0 + t * dx - px;
                let qy = y0 + t * dy - py;
                if qx * qx + qy * qy <= r * r {
                    if a >= 1.0 {
                        self.put(x, y, c);
                    } else {
                        self.blend(x, y, c, a);
                    }
                }
            }
        }
    }

    /// Oriented rectangle (car body).
    pub fn oriented_rect(
        &mut self,
        cx: f32,
        cy: f32,
        len: f32,
        wid: f32,
        heading: f32,
        c: Rgb,
        a: f32,
    ) {
        let (ch, sh) = (heading.cos(), heading.sin());
        let r = (len * len + wid * wid).sqrt() / 2.0;
        for y in (cy - r).floor() as i64..=(cy + r).ceil() as i64 {
            for x in (cx - r).floor() as i64..=(cx + r).ceil() as i64 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let u = dx * ch + dy * sh;
                let v = -dx * sh + dy * ch;
                if u.abs() <= len / 2.0 && v.abs() <= wid / 2.0 {
                    if a >= 1.0 {
                        self.put(x, y, c);
                    } else {
                        self.blend(x, y, c, a);
                    }
                }
            }
        }
    }

    pub fn text(&mut self, x: i64, y: i64, s: &str, scale: usize, c: Rgb) {
        let mut cx = x;
        for ch in s.chars() {
            let g = glyph(ch);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..GLYPH_W {
                    if bits & (1 << (GLYPH_W - 1 - col)) != 0 {
                        self.rect(
                            cx + (col * scale) as i64,
                            y + (row * scale) as i64,
                            scale as i64,
                            scale as i64,
                            c,
                        );
                    }
                }
            }
            cx += ((GLYPH_W + 1) * scale) as i64;
        }
    }

    pub fn text_centered(&mut self, y: i64, s: &str, scale: usize, c: Rgb) {
        let w = text_width(s, scale) as i64;
        self.text((self.w as i64 - w) / 2, y, s, scale, c);
    }

    /// Blit a window of `src` (top-left at sx, sy in src coords) to (dx, dy).
    pub fn blit(&mut self, src: &Frame, sx: i64, sy: i64, dx: i64, dy: i64, w: i64, h: i64) {
        for y in 0..h {
            let syy = sy + y;
            let dyy = dy + y;
            if dyy < 0 || dyy >= self.h as i64 {
                continue;
            }
            for x in 0..w {
                let sxx = sx + x;
                let dxx = dx + x;
                if dxx < 0 || dxx >= self.w as i64 {
                    continue;
                }
                if sxx >= 0 && syy >= 0 && (sxx as usize) < src.w && (syy as usize) < src.h {
                    let si = (syy as usize * src.w + sxx as usize) * 3;
                    let di = (dyy as usize * self.w + dxx as usize) * 3;
                    self.px[di..di + 3].copy_from_slice(&src.px[si..si + 3]);
                }
            }
        }
    }
}

pub fn glyph_h(scale: usize) -> i64 {
    (GLYPH_H * scale) as i64
}

/// Pre-rasterised track in world space at a fixed scale.
pub struct World {
    pub img: Frame,
    pub ppm: f32,
    pub minx: f32,
    pub miny: f32,
}

impl World {
    pub fn new(track: &Track, ppm: f32, margin_m: f32) -> World {
        let xs: Vec<f32> = track.nodes.iter().map(|n| n.x).collect();
        let ys: Vec<f32> = track.nodes.iter().map(|n| n.y).collect();
        let minx = xs.iter().cloned().fold(f32::INFINITY, f32::min) - margin_m;
        let maxx = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max) + margin_m;
        let miny = ys.iter().cloned().fold(f32::INFINITY, f32::min) - margin_m;
        let maxy = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max) + margin_m;
        let w = ((maxx - minx) * ppm).ceil().clamp(64.0, 12000.0) as usize;
        let h = ((maxy - miny) * ppm).ceil().clamp(64.0, 12000.0) as usize;
        let mut img = Frame::new(w, h, BG);
        let mut world = World {
            img,
            ppm,
            minx,
            miny,
        };
        // walls first (wider), then surface
        for pass in 0..2 {
            for i in 0..track.nodes.len() - 1 {
                let a = track.nodes[i];
                let b = track.nodes[i + 1];
                let (ax, ay) = world.to_px(a.x, a.y);
                let (bx, by) = world.to_px(b.x, b.y);
                let hw = a.half_width * ppm;
                if pass == 0 {
                    if track.walls {
                        world.img.line(ax, ay, bx, by, hw * 2.0 + 3.0, WALL, 1.0);
                    }
                } else {
                    let c = surface_colour(a.surface);
                    world.img.line(ax, ay, bx, by, hw * 2.0, c, 1.0);
                }
            }
        }
        // checkpoints & finish
        for &c in &track.checkpoints {
            let n = track.nodes[c];
            let (x, y) = world.to_px(n.x, n.y);
            world.img.disc(x, y, 1.6 * ppm, [240, 179, 90], 0.9);
        }
        let f = track.nodes[track.finish_node()];
        let (x, y) = world.to_px(f.x, f.y);
        world.img.line(
            x - 2.0 * ppm,
            y,
            x + 2.0 * ppm,
            y,
            1.0 * ppm,
            [255, 255, 255],
            0.9,
        );
        let s = track.nodes[0];
        let (x, y) = world.to_px(s.x, s.y);
        world.img.disc(x, y, 1.4 * ppm, GOOD, 0.9);
        img = world.img;
        World {
            img,
            ppm,
            minx,
            miny,
        }
    }

    #[inline]
    pub fn to_px(&self, x: f32, y: f32) -> (f32, f32) {
        // y up in world → down in image
        (
            (x - self.minx) * self.ppm,
            (self.img.h as f32) - (y - self.miny) * self.ppm,
        )
    }
}

pub fn fmt_time(ms: u32) -> String {
    let s = ms / 1000;
    let m = s / 60;
    if m > 0 {
        format!("{}:{:02}.{:03}", m, s % 60, ms % 1000)
    } else {
        format!("{}.{:03}", s, ms % 1000)
    }
}

fn state_at(states: &[CarState], tick: u32) -> Option<&CarState> {
    if states.is_empty() {
        return None;
    }
    let i = (tick as usize).min(states.len() - 1);
    Some(&states[i])
}

/// Everything needed to render one job.
pub struct Scene<'a> {
    pub job: &'a MediaJob,
    pub track: &'a Track,
    pub new: &'a Trajectory,
    pub old: Option<&'a Trajectory>,
    pub comparison: Option<&'a Comparison>,
    pub reference_label: Option<String>,
}

/// Render all frames, calling `sink` for each. Returns frame count.
pub fn render_frames(
    scene: &Scene,
    mut sink: impl FnMut(&Frame) -> anyhow::Result<()>,
) -> anyhow::Result<usize> {
    let job = scene.job;
    let (w, h) = (job.width as usize, job.height as usize);
    let fps = job.fps.max(1);
    let ppm = match job.camera {
        CameraDef::TopDownChase { pixels_per_meter } => pixels_per_meter,
        CameraDef::Overview => {
            let g = TrackGeom::new(scene.track.clone());
            let _ = g;
            let xs: Vec<f32> = scene.track.nodes.iter().map(|n| n.x).collect();
            let ys: Vec<f32> = scene.track.nodes.iter().map(|n| n.y).collect();
            let span = (xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
                - xs.iter().cloned().fold(f32::INFINITY, f32::min))
            .max(
                ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
                    - ys.iter().cloned().fold(f32::INFINITY, f32::min),
            );
            (w as f32 * 0.9 / span.max(1.0)).clamp(0.5, 20.0)
        }
    };
    let world = World::new(scene.track, ppm, 60.0);
    let mut count = 0usize;
    let mut emit = |f: &Frame| -> anyhow::Result<()> {
        sink(f)?;
        count += 1;
        Ok(())
    };

    let delta = scene
        .old
        .map(|o| scene.new.result.time_ms as i32 - o.result.time_ms as i32);

    // ---- title card
    if job.overlay.title_card {
        let mut f = Frame::new(w, h, BG);
        let headline = if scene.old.is_some() && delta.unwrap_or(0) < 0 {
            "NEW BEST TIME"
        } else {
            "NEW RUN"
        };
        f.text_centered((h as f32 * 0.32) as i64, headline, 10, TEXT);
        f.text_centered((h as f32 * 0.32) as i64 + 110, &scene.track.name, 6, DIM);
        if let Some(o) = scene.old {
            let line = format!(
                "{} > {}",
                fmt_time(o.result.time_ms),
                fmt_time(scene.new.result.time_ms)
            );
            f.text_centered((h as f32 * 0.46) as i64, &line, 8, TEXT);
            let d = delta.unwrap_or(0);
            f.text_centered(
                (h as f32 * 0.46) as i64 + 100,
                &format!("({:+.3} s)", d as f32 / 1000.0),
                8,
                if d < 0 { GOOD } else { BAD },
            );
        } else {
            f.text_centered(
                (h as f32 * 0.46) as i64,
                &fmt_time(scene.new.result.time_ms),
                10,
                TEXT,
            );
        }
        if let Some(r) = &scene.reference_label {
            f.text_centered((h as f32 * 0.58) as i64, r, 5, DIM);
        }
        f.text_centered(
            (h as f32 * 0.9) as i64,
            "RTI  recursive trackmania intelligence",
            4,
            DIM,
        );
        for _ in 0..(2 * fps) {
            emit(&f)?;
        }
    }

    // ---- part 1: new run (ghost = old)
    render_run(
        &world,
        scene,
        RunView {
            main: scene.new,
            ghost: scene.old,
            main_color: NEW,
            ghost_color: OLD,
            main_label: &job.new_label,
            ghost_label: &job.old_label,
            part_title: "PART 1  NEW RUN",
            slow: None,
        },
        &mut emit,
    )?;

    // ---- part 2: old run with new as ghost
    if job.comparison == ComparisonMode::Ghost {
        if let Some(old) = scene.old {
            render_run(
                &world,
                scene,
                RunView {
                    main: old,
                    ghost: Some(scene.new),
                    main_color: OLD,
                    ghost_color: NEW,
                    main_label: &job.old_label,
                    ghost_label: &job.new_label,
                    part_title: "PART 2  PREVIOUS BEST + GHOST",
                    slow: None,
                },
                &mut emit,
            )?;
        }
    }

    // ---- why it works: slow motion around the biggest gain
    if job.overlay.why_it_works {
        if let (Some(cmp), Some(old)) = (scene.comparison, scene.old) {
            if let Some(g) = &cmp.biggest_gain {
                let mut f = Frame::new(w, h, BG);
                f.text_centered((h as f32 * 0.4) as i64, "WHY IT WORKS", 10, TEXT);
                f.text_centered(
                    (h as f32 * 0.4) as i64 + 110,
                    &format!(
                        "{:.0}-{:.0} m   {} ms gained",
                        g.from_m, g.to_m, -g.section_delta_ms
                    ),
                    5,
                    DIM,
                );
                for _ in 0..(fps * 3 / 2) {
                    emit(&f)?;
                }
                render_run(
                    &world,
                    scene,
                    RunView {
                        main: scene.new,
                        ghost: Some(old),
                        main_color: NEW,
                        ghost_color: OLD,
                        main_label: &job.new_label,
                        ghost_label: &job.old_label,
                        part_title: "SLOW MOTION  0.25x",
                        slow: Some((g.from_m - 15.0, g.to_m + 15.0)),
                    },
                    &mut emit,
                )?;
            }
        }
    }

    // ---- difference card
    if job.overlay.difference_card {
        let mut f = Frame::new(w, h, BG);
        f.text_centered((h as f32 * 0.22) as i64, "THE DIFFERENCE", 10, TEXT);
        let mut y = (h as f32 * 0.22) as i64 + 160;
        if let Some(cmp) = scene.comparison {
            for b in cmp.bullets.iter().take(6) {
                f.text_centered(y, b, 6, TEXT);
                y += 70;
            }
        } else {
            f.text_centered(
                y,
                &format!(
                    "{} finished in {}",
                    job.new_label,
                    fmt_time(scene.new.result.time_ms)
                ),
                6,
                TEXT,
            );
        }
        if let Some(d) = delta {
            f.text_centered(
                y + 60,
                &format!("{:+} ms", d),
                12,
                if d < 0 { GOOD } else { BAD },
            );
        }
        f.text_centered(
            (h as f32 * 0.9) as i64,
            "RTI  recursive trackmania intelligence",
            4,
            DIM,
        );
        for _ in 0..(3 * fps) {
            emit(&f)?;
        }
    }
    Ok(count)
}

struct RunView<'a> {
    main: &'a Trajectory,
    ghost: Option<&'a Trajectory>,
    main_color: Rgb,
    ghost_color: Rgb,
    main_label: &'a str,
    ghost_label: &'a str,
    part_title: &'a str,
    /// (from_m, to_m) window rendered at quarter speed.
    slow: Option<(f32, f32)>,
}

fn render_run(
    world: &World,
    scene: &Scene,
    view: RunView,
    emit: &mut impl FnMut(&Frame) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let job = scene.job;
    let (w, h) = (job.width as usize, job.height as usize);
    let fps = job.fps.max(1) as f32;
    let states = &view.main.states;
    if states.is_empty() {
        return Ok(());
    }
    let total_ticks = states.len() as u32 - 1;
    // ticks per frame: real time = 100 ticks/s
    let mut tpf = 100.0 / fps;
    let real_secs = total_ticks as f32 / 100.0;
    if view.slow.is_none() && real_secs > job.overlay.max_part_seconds {
        tpf *= real_secs / job.overlay.max_part_seconds;
    }
    let (start_tick, end_tick, tpf) = match view.slow {
        Some((from, to)) => {
            let a = states.iter().position(|s| s.progress >= from).unwrap_or(0) as u32;
            let b = states
                .iter()
                .position(|s| s.progress >= to)
                .unwrap_or(states.len() - 1) as u32;
            (a, b.max(a + 1), tpf * 0.25)
        }
        None => (0, total_ticks, tpf),
    };
    let view_h = (h as f32 * 0.72) as i64;
    let hud_y = view_h;
    let mut t = start_tick as f32;
    let mut frame_i = 0usize;
    while t <= end_tick as f32 {
        let tick = t.round() as u32;
        let s = state_at(states, tick).unwrap();
        let mut f = Frame::new(w, h, BG);
        // camera window centred on the car
        let (cx, cy) = world.to_px(s.x, s.y);
        let sx = cx as i64 - w as i64 / 2;
        let sy = cy as i64 - view_h / 2;
        f.blit(&world.img, sx, sy, 0, 0, w as i64, view_h);
        // trails
        let draw_trail = |f: &mut Frame, tr: &Trajectory, color: Rgb, upto: u32, alpha: f32| {
            let n = (upto as usize).min(tr.states.len());
            let step = 3usize;
            let mut i = n.saturating_sub(400);
            while i + step < n {
                let a = tr.states[i];
                let b = tr.states[i + step];
                let (ax, ay) = world.to_px(a.x, a.y);
                let (bx, by) = world.to_px(b.x, b.y);
                f.line(
                    ax - sx as f32,
                    ay - sy as f32,
                    bx - sx as f32,
                    by - sy as f32,
                    3.0,
                    color,
                    alpha,
                );
                i += step;
            }
        };
        if let Some(g) = view.ghost {
            draw_trail(&mut f, g, view.ghost_color, tick, 0.35);
        }
        draw_trail(&mut f, view.main, view.main_color, tick, 0.8);
        // ghost car
        if let Some(g) = view.ghost {
            if let Some(gs) = state_at(&g.states, tick) {
                let (gx, gy) = world.to_px(gs.x, gs.y);
                f.oriented_rect(
                    gx - sx as f32,
                    gy - sy as f32,
                    4.2 * world.ppm,
                    2.0 * world.ppm,
                    -gs.heading,
                    view.ghost_color,
                    0.45,
                );
            }
        }
        // main car
        f.oriented_rect(
            cx - sx as f32,
            cy - sy as f32,
            4.2 * world.ppm,
            2.0 * world.ppm,
            -s.heading,
            view.main_color,
            1.0,
        );
        f.disc(
            cx - sx as f32 + (s.heading.cos()) * 1.6 * world.ppm,
            cy - sy as f32 - (s.heading.sin()) * 1.6 * world.ppm,
            0.5 * world.ppm,
            [255, 255, 255],
            1.0,
        );
        // HUD
        if job.overlay.hud {
            f.rect(0, hud_y, w as i64, h as i64 - hud_y, [20, 23, 31]);
            f.rect(0, hud_y, w as i64, 4, view.main_color);
            f.text(40, hud_y + 30, view.part_title, 5, DIM);
            let time_s = fmt_time(s.tick * rti_core::TICK_MS);
            f.text(40, hud_y + 100, &time_s, 12, TEXT);
            f.text(
                40,
                hud_y + 210,
                &format!("{:>3.0} km/h", s.speed() * 3.6),
                7,
                TEXT,
            );
            // legend
            f.rect(40, hud_y + 300, 40, 40, view.main_color);
            f.text(100, hud_y + 305, view.main_label, 5, TEXT);
            if let Some(g) = view.ghost {
                f.rect(40, hud_y + 360, 40, 40, view.ghost_color);
                f.text(100, hud_y + 365, view.ghost_label, 5, TEXT);
                // live delta vs ghost at the same progress
                if let Some(gt) = g
                    .states
                    .iter()
                    .find(|x| x.progress >= s.progress)
                    .map(|x| x.tick)
                {
                    let d = (s.tick as i32 - gt as i32) * rti_core::TICK_MS as i32;
                    let col = if d < 0 {
                        GOOD
                    } else if d > 0 {
                        BAD
                    } else {
                        DIM
                    };
                    let txt = format!("{:+.3}", d as f32 / 1000.0);
                    let tw = text_width(&txt, 9) as i64;
                    f.text(w as i64 - 40 - tw, hud_y + 100, &txt, 9, col);
                    let lbl = "vs ghost";
                    let lw = text_width(lbl, 4) as i64;
                    f.text(w as i64 - 40 - lw, hud_y + 190, lbl, 4, DIM);
                }
            }
            if s.wall_hits > 0 {
                let txt = format!("walls {}", s.wall_hits);
                let tw = text_width(&txt, 4) as i64;
                f.text(w as i64 - 40 - tw, hud_y + 305, &txt, 4, DIM);
            }
            let _ = glyph_h(1);
        }
        emit(&f)?;
        frame_i += 1;
        t += tpf;
        if frame_i > 20_000 {
            break;
        }
    }
    // hold the last frame briefly
    Ok(())
}

/// Convenience: a single overview PNG-like frame (RGB) of the track with both
/// trajectories, for thumbnails and the UI.
pub fn overview_frame(
    track: &Track,
    new: &Trajectory,
    old: Option<&Trajectory>,
    w: usize,
    h: usize,
) -> Frame {
    let xs: Vec<f32> = track.nodes.iter().map(|n| n.x).collect();
    let ys: Vec<f32> = track.nodes.iter().map(|n| n.y).collect();
    let span_x = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
        - xs.iter().cloned().fold(f32::INFINITY, f32::min)
        + 60.0;
    let span_y = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
        - ys.iter().cloned().fold(f32::INFINITY, f32::min)
        + 60.0;
    let ppm = (w as f32 / span_x).min(h as f32 / span_y).clamp(0.2, 20.0);
    let world = World::new(track, ppm, 30.0);
    let mut f = Frame::new(w, h, BG);
    let ox = (w as i64 - world.img.w as i64) / 2;
    let oy = (h as i64 - world.img.h as i64) / 2;
    f.blit(
        &world.img,
        0,
        0,
        ox,
        oy,
        world.img.w as i64,
        world.img.h as i64,
    );
    let mut draw = |tr: &Trajectory, c: Rgb| {
        for pair in tr.states.windows(4).step_by(3) {
            let (ax, ay) = world.to_px(pair[0].x, pair[0].y);
            let (bx, by) = world.to_px(pair[3].x, pair[3].y);
            f.line(
                ax + ox as f32,
                ay + oy as f32,
                bx + ox as f32,
                by + oy as f32,
                2.5,
                c,
                0.9,
            );
        }
    };
    if let Some(o) = old {
        draw(o, OLD);
    }
    draw(new, NEW);
    f
}
