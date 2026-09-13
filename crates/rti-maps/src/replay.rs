//! .Replay.Gbx decoding (CGameCtnReplayRecord): the embedded map, ghost
//! metadata (login, race time, checkpoint times) and — the valuable part —
//! the player's per-tick inputs (chunk 0x0309201D, TM2020 bitstream format,
//! algorithm after GBX.NET, credit Mystixor/Shweetz/BigBang1112).
//!
//! TM2020 replays store no position samples: the game regenerates the ghost
//! from inputs because its physics is deterministic. That makes a replay an
//! exact human input trajectory RTI can warm-start from and compare against.

use rti_core::Action;
use serde::{Deserialize, Serialize};

use crate::container::{read_container, scan_next_skippable, zlib_inflate, FACADE, SKIP};
use crate::gbx::{parse_map, ParsedMap};
use crate::reader::{GbxError, GbxResult, IdState, Reader};

pub const CLASS_REPLAY: u32 = 0x0309_3000;
pub const CLASS_GHOST: u32 = 0x0309_2000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputEvent {
    /// Steering in [-127, 127] (negative = left).
    Steer {
        value: i8,
    },
    Accelerate {
        on: bool,
    },
    Brake {
        on: bool,
    },
    Horn {
        on: bool,
    },
    Respawn,
    SecondaryRespawn,
    /// Raw 34-bit state word (kept for research; bit meanings partly unknown).
    States {
        value: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TimedInput {
    pub time_ms: i32,
    pub event: InputEvent,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Ghost {
    pub login: String,
    pub nickname: String,
    pub race_time_ms: u32,
    pub checkpoint_times_ms: Vec<u32>,
    pub respawns: u32,
    pub input_version: u32,
    pub start_offset_ms: i32,
    pub ticks: u32,
    pub inputs: Vec<TimedInput>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ParsedReplay {
    pub map_uid: String,
    pub map_name: String,
    pub player_nickname: String,
    pub player_login: String,
    pub time_ms: u32,
    pub ghosts: Vec<Ghost>,
    #[serde(skip)]
    pub map_bytes: Vec<u8>,
    #[serde(skip)]
    pub map: Option<ParsedMap>,
    pub warnings: Vec<String>,
    pub source_hash: String,
}

/// LSB-first bit reader (GBX.NET semantics).
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn bit(&mut self) -> GbxResult<bool> {
        let byte = self.pos / 8;
        if byte >= self.data.len() {
            return Err(GbxError::Eof(byte, 1));
        }
        let b = (self.data[byte] >> (self.pos % 8)) & 1 != 0;
        self.pos += 1;
        Ok(b)
    }
    fn number(&mut self, bits: usize) -> GbxResult<u64> {
        let mut v = 0u64;
        for i in 0..bits {
            if self.bit()? {
                v |= 1 << i;
            }
        }
        Ok(v)
    }
    fn remaining_zero(&self) -> bool {
        let mut p = self.pos;
        while p < self.data.len() * 8 {
            if (self.data[p / 8] >> (p % 8)) & 1 != 0 {
                return false;
            }
            p += 1;
        }
        true
    }
}

/// Decode a TM2020 (version 11/12) input bitstream into timed events.
pub fn decode_inputs(
    data: &[u8],
    ticks: u32,
    version: u32,
    start_offset_ms: i32,
) -> (Vec<TimedInput>, Vec<String>) {
    let mut out = vec![];
    let mut warnings = vec![];
    let mut r = BitReader { data, pos: 0 };
    let mut started = 0u8; // 0 = not started, 2 = vehicle, 1 = character
    let mut prev_steer = 0i8;
    let mut prev_accel = false;
    let mut prev_brake = false;
    let mut prev_horn = false;
    let res: GbxResult<()> = (|| {
        for i in 0..ticks {
            let time = i as i32 * 10 + start_offset_ms;
            let same_state = r.bit()?;
            if !same_state {
                let only2 = r.bit()?;
                if only2 {
                    let states = r.number(2)?;
                    if started == 2 {
                        let horn = states & 2 != 0;
                        if horn != prev_horn {
                            out.push(TimedInput {
                                time_ms: time,
                                event: InputEvent::Horn { on: horn },
                            });
                            prev_horn = horn;
                        }
                    }
                } else {
                    let states = r.number(if version == 11 { 33 } else { 34 })?;
                    started = match states & 0xF {
                        0 => 0,
                        1 => 1,
                        2 | 3 | 4 => 2,
                        0xC | 0xD => {
                            return Err(GbxError::Format(
                                "float-based steering inputs are not supported".into(),
                            ))
                        }
                        _ => 1,
                    };
                    let horn = states & (1 << 6) != 0;
                    if started == 2 && horn != prev_horn {
                        out.push(TimedInput {
                            time_ms: time,
                            event: InputEvent::Horn { on: horn },
                        });
                        prev_horn = horn;
                    }
                    if (states >> 31) & 1 != 0 {
                        out.push(TimedInput {
                            time_ms: time,
                            event: InputEvent::Respawn,
                        });
                    }
                    if (states >> 33) & 1 != 0 {
                        out.push(TimedInput {
                            time_ms: time,
                            event: InputEvent::SecondaryRespawn,
                        });
                    }
                    out.push(TimedInput {
                        time_ms: time,
                        event: InputEvent::States { value: states },
                    });
                }
            }
            let same_mouse = r.bit()?;
            if !same_mouse {
                let _x = r.number(16)?;
                let _y = r.number(16)?;
            }
            match started {
                2 => {
                    let same = r.bit()?;
                    if same {
                        continue;
                    }
                    let steer = r.number(8)? as u8 as i8;
                    if steer != prev_steer {
                        out.push(TimedInput {
                            time_ms: time,
                            event: InputEvent::Steer { value: steer },
                        });
                        prev_steer = steer;
                    }
                    let accel = r.bit()?;
                    if accel != prev_accel {
                        out.push(TimedInput {
                            time_ms: time,
                            event: InputEvent::Accelerate { on: accel },
                        });
                        prev_accel = accel;
                    }
                    let brake = r.bit()?;
                    if brake != prev_brake {
                        out.push(TimedInput {
                            time_ms: time,
                            event: InputEvent::Brake { on: brake },
                        });
                        prev_brake = brake;
                    }
                }
                1 => {
                    let same = r.bit()?;
                    if same {
                        continue;
                    }
                    let _ = r.number(8)?; // strafe, walk, vertical, horizontal (2 bits each)
                }
                _ => {}
            }
        }
        Ok(())
    })();
    if let Err(e) = res {
        warnings.push(format!("input decode stopped: {e}"));
    } else if !r.remaining_zero() {
        warnings.push("input buffer not fully consumed (trailing non-zero bits)".into());
    }
    (out, warnings)
}

/// Expand timed events into a per-tick action sequence of `ticks` length.
/// Steering is mapped from [-127, 127] to [-1, 1]. Ticks before the race
/// start (negative times) are dropped.
pub fn inputs_to_actions(inputs: &[TimedInput], ticks: u32) -> Vec<Action> {
    let mut out = Vec::with_capacity(ticks as usize);
    let mut cur = Action::coast();
    let mut idx = 0usize;
    for t in 0..ticks {
        let time = t as i32 * 10;
        while idx < inputs.len() && inputs[idx].time_ms <= time {
            match inputs[idx].event {
                InputEvent::Steer { value } => cur.steer = value as f32 / 127.0,
                InputEvent::Accelerate { on } => cur.gas = on,
                InputEvent::Brake { on } => cur.brake = on,
                _ => {}
            }
            idx += 1;
        }
        out.push(cur);
    }
    out
}

pub fn parse_replay(data: &[u8]) -> anyhow::Result<ParsedReplay> {
    let c = read_container(data)?;
    anyhow::ensure!(
        c.class_id == CLASS_REPLAY,
        "not a replay (class 0x{:08X})",
        c.class_id
    );
    let mut out = ParsedReplay {
        source_hash: blake3::hash(data).to_hex().to_string(),
        ..Default::default()
    };
    let mut hid = IdState::default();
    for (id, chunk) in &c.header_chunks {
        if let Err(e) = parse_header(*id, chunk, &mut out, &mut hid) {
            out.warnings.push(format!("header 0x{id:08X}: {e}"));
        }
    }
    let body = &c.body;
    let mut r = Reader::new(body);
    let mut ids = IdState::default();
    while let Ok(id) = r.u32() {
        if id == FACADE {
            break;
        }
        if r.peek_u32().ok() == Some(SKIP) {
            let _ = r.u32();
            let size = r.u32()? as usize;
            r.skip(size)?;
            continue;
        }
        let res: GbxResult<bool> = (|| {
            match id {
                0x0309_3002 => {
                    let size = r.u32()? as usize;
                    out.map_bytes = r.bytes(size)?.to_vec();
                    Ok(true)
                }
                0x0309_3014 => {
                    let _version = r.u32()?;
                    let count = r.u32()?;
                    for _ in 0..count {
                        let idx = r.u32()?;
                        if idx == 0xFFFF_FFFF {
                            continue;
                        }
                        let class = r.u32()?;
                        if class != CLASS_GHOST {
                            return Err(GbxError::Format(format!(
                                "ghost node has class 0x{class:08X}"
                            )));
                        }
                        let g = parse_ghost(&mut r, &mut ids)?;
                        out.ghosts.push(g);
                    }
                    Ok(false) // stop: everything after is skippable or unneeded
                }
                0x0309_3015 => {
                    let idx = r.u32()?;
                    if idx != 0xFFFF_FFFF {
                        return Err(GbxError::Format("media clip present".into()));
                    }
                    Ok(true)
                }
                _ => Err(GbxError::Format(format!("unknown replay chunk 0x{id:08X}"))),
            }
        })();
        match res {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => match scan_next_skippable(body, r.pos, 0x0309_3000) {
                Some(n) => {
                    out.warnings
                        .push(format!("chunk 0x{id:08X}: {e}; resumed at {n}"));
                    r.pos = n;
                }
                None => {
                    out.warnings
                        .push(format!("stopped at chunk 0x{id:08X}: {e}"));
                    break;
                }
            },
        }
    }
    if !out.map_bytes.is_empty() {
        match parse_map(&out.map_bytes) {
            Ok(m) => {
                if out.map_uid.is_empty() {
                    out.map_uid = m.info.uid.clone();
                }
                if out.map_name.is_empty() {
                    out.map_name = m.info.name.clone();
                }
                out.map = Some(m);
            }
            Err(e) => out.warnings.push(format!("embedded map: {e}")),
        }
    }
    Ok(out)
}

fn parse_header(id: u32, chunk: &[u8], out: &mut ParsedReplay, ids: &mut IdState) -> GbxResult<()> {
    let mut r = Reader::new(chunk);
    match id {
        0x0309_3000 => {
            let v = r.u32()?;
            if v >= 2 {
                let (uid, _, _) = r.ident(ids)?;
                out.map_uid = uid;
                out.time_ms = r.u32()?;
                out.player_nickname = r.string()?;
            }
            if v >= 6 {
                out.player_login = r.string()?;
            }
        }
        0x0309_3002 => {
            let _v = r.u32()?;
            let _author_version = r.u32()?;
            let login = r.string()?;
            if out.player_login.is_empty() {
                out.player_login = login;
            }
            let nick = r.string()?;
            if out.player_nickname.is_empty() {
                out.player_nickname = nick;
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_ghost(r: &mut Reader, ids: &mut IdState) -> GbxResult<Ghost> {
    let mut g = Ghost::default();
    loop {
        let id = r.u32()?;
        if id == FACADE {
            break;
        }
        if r.peek_u32()? == SKIP {
            let _ = r.u32()?;
            let size = r.u32()? as usize;
            let chunk = r.bytes(size)?;
            let mut c = Reader::new(chunk);
            match id {
                0x0309_2000 => {
                    // version + playerModel Ident: reads the body's lookback version
                    let _v = c.u32()?;
                    let _ = c.ident(ids)?;
                }
                0x0309_2005 => g.race_time_ms = c.u32()?,
                0x0309_2008 => g.respawns = c.u32()?,
                0x0309_200B => {
                    let n = c.u32()?;
                    for _ in 0..n {
                        let t = c.u32()?;
                        let _stunts = c.u32()?;
                        g.checkpoint_times_ms.push(t);
                    }
                }
                0x0309_201D => {
                    let chunk_version = c.u32()?;
                    let n = c.u32()?;
                    for _ in 0..n {
                        let version = c.u32()?;
                        let _u = c.u32()?;
                        let start_offset = if chunk_version >= 4 { c.i32()? } else { 0 };
                        let ticks = c.u32()?;
                        let len = c.u32()? as usize;
                        let data = c.bytes(len)?;
                        g.input_version = version;
                        g.start_offset_ms = if start_offset == -1 { 0 } else { start_offset };
                        g.ticks = ticks;
                        if version >= 11 {
                            let (inputs, w) =
                                decode_inputs(data, ticks, version, g.start_offset_ms);
                            g.inputs = inputs;
                            g.warnings.extend(w);
                        } else {
                            g.warnings
                                .push(format!("input version {version} (pre-TM2020) not decoded"));
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        match id {
            0x0303_F005 => {
                let unc = r.u32()? as usize;
                let comp = r.u32()? as usize;
                let src = r.bytes(comp)?;
                let _ = zlib_inflate(src, unc);
            }
            0x0303_F006 => {
                let _replaying = r.u32()?;
                let unc = r.u32()? as usize;
                let comp = r.u32()? as usize;
                let src = r.bytes(comp)?;
                let _ = zlib_inflate(src, unc);
            }
            0x0309_200C | 0x0309_2014 => {
                let _ = r.u32()?;
            }
            0x0309_200E => {
                let _uid = r.u32()?;
            }
            0x0309_200F => g.login = r.string()?,
            0x0309_2010 => {
                let _ = r.id(ids)?;
            }
            0x0309_2012 => {
                let _ = r.u32()?;
                let _ = r.u64()?;
                let _ = r.u64()?;
            }
            0x0309_2018 => {
                let _ = r.ident(ids)?;
            }
            0x0309_201C => {
                r.skip(32)?;
            }
            0x0309_2009 => {
                let _ = r.vec3()?;
            }
            other => {
                return Err(GbxError::Format(format!(
                    "unknown ghost chunk 0x{other:08X}"
                )))
            }
        }
    }
    Ok(g)
}
