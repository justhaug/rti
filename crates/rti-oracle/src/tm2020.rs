//! Client for the TM2020 bridge. The game side (Openplanet plugin running
//! under Proton) is user-provided; see docs/oracle.md for the contract.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use rti_core::action::compress_actions;
use rti_core::{Action, CarState, RunResult, Track, TICK_MS};

use crate::protocol::{Request, Response, TmState, PROTOCOL_VERSION};
use crate::{Oracle, OracleRun};

#[derive(Clone, Debug)]
pub struct Tm2020 {
    pub host: String,
    pub port: u16,
    pub timeout: Duration,
}

impl Tm2020 {
    pub fn new(host: &str, port: u16, timeout_secs: u64) -> Tm2020 {
        Tm2020 {
            host: host.to_string(),
            port,
            timeout: Duration::from_secs(timeout_secs.max(1)),
        }
    }

    fn connect(&self) -> anyhow::Result<Conn> {
        let addr = format!("{}:{}", self.host, self.port);
        let sock = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow::anyhow!("could not resolve {addr}"))?;
        let stream = TcpStream::connect_timeout(&sock, Duration::from_secs(5)).map_err(|e| {
            anyhow::anyhow!("TM2020 bridge not reachable at {addr}: {e}. Start the game + Openplanet bridge plugin (docs/oracle.md) or set [oracle] kind = \"hidden_sim\".")
        })?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut conn = Conn { stream, reader };
        let hello = conn.call(&Request::Hello {
            protocol: PROTOCOL_VERSION,
        })?;
        if hello.protocol != Some(PROTOCOL_VERSION) {
            anyhow::bail!("bridge protocol {:?} != {PROTOCOL_VERSION}", hello.protocol);
        }
        Ok(conn)
    }
}

struct Conn {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Conn {
    fn call(&mut self, req: &Request) -> anyhow::Result<Response> {
        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        self.stream.write_all(line.as_bytes())?;
        self.stream.flush()?;
        let mut buf = String::new();
        let n = self.reader.read_line(&mut buf)?;
        if n == 0 {
            anyhow::bail!("bridge closed the connection");
        }
        let resp: Response = serde_json::from_str(buf.trim_end())?;
        if !resp.ok {
            anyhow::bail!(
                "bridge error: {}",
                resp.error.unwrap_or_else(|| "unknown".into())
            );
        }
        Ok(resp)
    }
}

/// Convert game telemetry into the planar `CarState` used by the sim,
/// projecting through the track's `tm_frame` and locating on the track.
pub fn project_states(track: &Track, states: &[TmState]) -> Vec<CarState> {
    let frame = track.tm_frame.unwrap_or_default();
    let geom = rti_sim::TrackGeom::new(track.clone());
    let mut out = Vec::with_capacity(states.len());
    let mut seg = 0usize;
    for s in states {
        let (x, y) = frame.to_local(s.pos);
        let (vx, vy) =
            { frame.to_local([s.vel[0] + frame.origin[0], 0.0, s.vel[2] + frame.origin[2]]) };
        let loc = geom.locate(x, y, seg);
        seg = loc.seg;
        out.push(CarState {
            tick: s.tick,
            x,
            y,
            heading: rti_sim::geom::wrap_angle(s.yaw - frame.yaw),
            vx,
            vy,
            progress: loc.progress,
            next_checkpoint: s.cp,
            finished: s.finished,
            finish_tick: if s.finished {
                s.race_time_ms / TICK_MS
            } else {
                0
            },
            seg: seg as u32,
            ..Default::default()
        });
    }
    out
}

impl Oracle for Tm2020 {
    fn name(&self) -> String {
        "tm2020".into()
    }

    fn available(&self) -> anyhow::Result<()> {
        let mut c = self.connect()?;
        c.call(&Request::Ping)?;
        Ok(())
    }

    fn run(&self, track: &Track, actions: &[Action], max_ticks: u32) -> anyhow::Result<OracleRun> {
        let uid = track.tm_map_uid.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "track {:?} has no tm_map_uid; cannot verify in TM2020",
                track.name
            )
        })?;
        let mut c = self.connect()?;
        c.call(&Request::LoadMap { uid })?;
        let resp = c.call(&Request::Run {
            inputs: compress_actions(actions),
            max_ticks,
            telemetry: true,
        })?;
        let r = resp.result.unwrap_or_default();
        let states = project_states(track, &resp.states);
        let progress = states.last().map(|s| s.progress).unwrap_or(0.0);
        let max_speed = states.iter().map(|s| s.speed()).fold(0.0, f32::max);
        let mean_speed = if states.is_empty() {
            0.0
        } else {
            states.iter().map(|s| s.speed()).sum::<f32>() / states.len() as f32
        };
        let result = RunResult {
            finished: r.finished,
            time_ms: if r.finished {
                r.race_time_ms
            } else {
                r.ticks * TICK_MS
            },
            ticks: r.ticks,
            checkpoints_hit: r.checkpoints,
            progress,
            max_speed,
            mean_speed,
            offtrack_ticks: 0,
            wall_hits: 0,
        };
        Ok(OracleRun {
            states,
            result,
            ticks: r.ticks as u64,
        })
    }

    fn ms_per_tick(&self) -> f64 {
        10.0
    }
}
