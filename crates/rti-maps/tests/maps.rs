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
        mk("RoadTechCheckpoint", 1, 12, 14),
        mk("RoadTechFinish", 1, 13, 14),
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
        (track.length() - (32.0 * 4.0 + std::f32::consts::FRAC_PI_2 * 16.0 + 32.0 * 3.0)).abs()
            < 12.0,
        "len {}",
        track.length()
    );
}
