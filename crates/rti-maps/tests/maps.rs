use rti_maps::catalog::Catalog;
use rti_maps::{compile_track, parse_map, TmxClient};

const SAMPLE: &[u8] = include_bytes!("data/how_to_map_356566.Map.Gbx");

#[test]
fn parses_sample_map() {
    let m = parse_map(SAMPLE).unwrap();
    assert_eq!(m.info.name, "How to map");
    assert_eq!(m.info.uid, "oHKMUVktceo8Ue1Q0_vJlzsiHUa");
    assert!(m.blocks.len() > 3000, "{}", m.blocks.len());
    assert!(m.blocks.iter().any(|b| b.name == "RoadTechStraight"));
    assert!(
        m.blocks
            .iter()
            .any(|b| b.waypoint_tag.as_deref() == Some("Goal")),
        "no finish waypoint"
    );
    assert_eq!(m.info.author_ms, 47624);
    assert!(m.info.xml.contains("<header"));
}

#[test]
fn compiles_sample_map_to_track() {
    let m = parse_map(SAMPLE).unwrap();
    let cat = Catalog::default_catalog();
    let (track, report) = compile_track(&m, &cat, "how_to_map").unwrap();
    eprintln!("{report:#?}");
    assert!(report.blocks_recognized >= 20, "{report:?}");
    assert!(report.blocks_chained >= 5, "{report:?}");
    assert!(track.nodes.len() >= 10);
    assert!(track.length() > 100.0);
    track.validate().unwrap();
    // it must be simulable
    let sim = rti_sim::Sim::new(
        rti_core::PhysicsParams::default(),
        rti_sim::TrackGeom::new(track),
    );
    let acts = vec![rti_core::Action::full_gas(); 300];
    let ro = rti_sim::rollout(&sim, &sim.initial_state(), &acts, 300, false);
    assert!(ro.result.progress > 5.0);
}

#[test]
fn tmx_id_parsing() {
    assert_eq!(TmxClient::parse_id("123"), Some(123));
    assert_eq!(TmxClient::parse_id("tmx:42"), Some(42));
    assert_eq!(
        TmxClient::parse_id("https://trackmania.exchange/maps/356566/how-to-map"),
        Some(356566)
    );
    assert_eq!(
        TmxClient::parse_id("https://trackmania.exchange/mapshow/99"),
        Some(99)
    );
    assert_eq!(TmxClient::parse_id("nope"), None);
}

/// Synthetic map placed with the verified convention: start → straight →
/// curve (top↔left) → straight → checkpoint → finish. Must chain fully.
#[test]
fn compiles_synthetic_chain() {
    use rti_maps::{MapBlock, MapInfo, ParsedMap};
    let mk = |name: &str, dir: u8, x: u8, z: u8| MapBlock {
        name: name.into(),
        dir,
        coord: [x, 5, z],
        flags: 0,
        ground: true,
        free: false,
        variant: 0,
        waypoint_tag: None,
        waypoint_order: None,
        skin: None,
        free_pos: None,
        free_pyr: None,
    };
    // dir 0 straights run along +z; a Curve1 at dir 2 joins its bottom edge to its right edge
    let blocks = vec![
        mk("RoadTechStart", 0, 10, 10),
        mk("RoadTechStraight", 0, 10, 11),
        mk("DecoWallBasePillar", 0, 10, 12), // unrecognised: bridged
        mk("RoadTechStraight", 0, 10, 13),
        mk("RoadTechCurve1", 2, 10, 14), // bottom (from z=14) → right edge (x=11)
        mk("RoadTechStraight", 1, 11, 14), // along x
        mk("RoadTechCheckpoint", 3, 12, 14),
        mk("RoadTechFinish", 3, 13, 14), // faces +x: entered through its back (left) edge
        mk("RoadTechStraight", 0, 30, 30), // stray piece, not part of the chain
    ];
    let map = ParsedMap {
        info: MapInfo {
            name: "synthetic".into(),
            ..Default::default()
        },
        blocks,
        ..Default::default()
    };
    let (track, report) = compile_track(&map, &Catalog::default_catalog(), "synthetic").unwrap();
    eprintln!("{report:?}");
    assert!(report.start_found && report.finish_found, "{report:?}");
    assert_eq!(report.blocks_chained, 7, "{report:?}");
    assert_eq!(report.bridged_gaps, 1);
    assert_eq!(track.checkpoints.len(), 1);
    assert!(track.finish.is_some());
    assert!(
        // the car spawns 30 m into the start block, so the compiled track is 30 m shorter
        (track.length() - (32.0 * 4.0 + std::f32::consts::FRAC_PI_2 * 16.0 + 32.0 * 3.0 - 30.0))
            .abs()
            < 12.0,
        "len {}",
        track.length()
    );
}

/// Bit-exact input decoding against a hand-built TM2020 input stream:
/// tick 0 sets the vehicle state word (kind 2), tick 3 presses accelerate,
/// tick 5 steers left, everything else "same".
#[test]
fn decodes_synthetic_input_stream() {
    use rti_maps::replay::{decode_inputs, inputs_to_actions, InputEvent};
    let mut bits: Vec<u8> = vec![];
    let mut push = |v: u64, n: usize| {
        for i in 0..n {
            bits.push(((v >> i) & 1) as u8);
        }
    };
    // tick 0: sameState=0, only2Bit=0, states(34 bits)=2, sameMouse=1, sameVehicleValue=1
    push(0, 1);
    push(0, 1);
    push(2, 34);
    push(1, 1);
    push(1, 1);
    // ticks 1,2: same everything (sameState=1, sameMouse=1, sameVehicleValue=1)
    for _ in 0..2 {
        push(1, 1);
        push(1, 1);
        push(1, 1);
    }
    // tick 3: same state, same mouse, vehicle value: steer 0, accel 1, brake 0
    push(1, 1);
    push(1, 1);
    push(0, 1);
    push(0, 8);
    push(1, 1);
    push(0, 1);
    // tick 4: same
    push(1, 1);
    push(1, 1);
    push(1, 1);
    // tick 5: steer -127 (0x81), accel 1, brake 0
    push(1, 1);
    push(1, 1);
    push(0, 1);
    push(0x81, 8);
    push(1, 1);
    push(0, 1);
    let mut data = vec![0u8; bits.len().div_ceil(8)];
    for (i, b) in bits.iter().enumerate() {
        data[i / 8] |= b << (i % 8);
    }
    let (events, warnings) = decode_inputs(&data, 6, 12, 0);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(events[0].event, InputEvent::States { value: 2 });
    assert_eq!(
        events[1],
        rti_maps::replay::TimedInput {
            time_ms: 30,
            event: InputEvent::Accelerate { on: true }
        }
    );
    assert_eq!(
        events[2],
        rti_maps::replay::TimedInput {
            time_ms: 50,
            event: InputEvent::Steer { value: -127 }
        }
    );
    let acts = inputs_to_actions(&events, 6);
    assert!(!acts[2].gas && acts[3].gas && acts[5].gas);
    assert!((acts[5].steer + 1.0).abs() < 1e-6 && acts[4].steer == 0.0);
}

#[test]
fn parses_real_replay_when_available() {
    let Ok(p) = std::env::var("RTI_TEST_REPLAY") else {
        return;
    };
    let r = rti_maps::parse_replay(&std::fs::read(p).unwrap()).unwrap();
    assert!(!r.ghosts.is_empty());
    let g = &r.ghosts[0];
    assert!(
        g.race_time_ms > 0 && g.ticks > 0 && !g.inputs.is_empty(),
        "{g:?}"
    );
    assert!(g.warnings.is_empty(), "{:?}", g.warnings);
    assert!(r.map.is_some(), "{:?}", r.warnings);
}
