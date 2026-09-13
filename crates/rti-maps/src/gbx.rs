//! CGameCtnChallenge (.Map.Gbx) decoding: container, header chunks, block
//! list, and (best effort) anchored items.

use serde::{Deserialize, Serialize};

use crate::reader::{GbxError, GbxResult, IdState, NodeState, Reader};

pub const CLASS_CHALLENGE: u32 = 0x0304_3000;
const FACADE: u32 = 0xFACA_DE01;
const SKIP: u32 = 0x534B_4950; // "PIKS"

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MapInfo {
    pub uid: String,
    pub name: String,
    pub author: String,
    pub author_nick: String,
    pub collection: String,
    pub decoration: String,
    pub map_type: String,
    pub map_style: String,
    pub title_id: String,
    pub bronze_ms: u32,
    pub silver_ms: u32,
    pub gold_ms: u32,
    pub author_ms: u32,
    pub nb_checkpoints: u32,
    pub nb_laps: u32,
    pub is_lap_race: bool,
    pub size: [u32; 3],
    pub xml: String,
    pub vehicle: String,
}

/// One block placement. Coordinates are in block units (32 m horizontal,
/// 8 m vertical); `dir` is 0..3 (quarter turns).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapBlock {
    pub name: String,
    pub dir: u8,
    pub coord: [u8; 3],
    pub flags: u32,
    #[serde(default)]
    pub ground: bool,
    #[serde(default)]
    pub free: bool,
    #[serde(default)]
    pub variant: u32,
    #[serde(default)]
    pub waypoint_tag: Option<String>,
    #[serde(default)]
    pub waypoint_order: Option<u32>,
    #[serde(default)]
    pub skin: Option<String>,
    /// Absolute position (metres) and pitch/yaw/roll for free blocks.
    #[serde(default)]
    pub free_pos: Option<[f32; 3]>,
    #[serde(default)]
    pub free_pyr: Option<[f32; 3]>,
}

/// One anchored item (custom or vanilla). Position in metres, world frame.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapItem {
    pub model: String,
    pub pos: [f32; 3],
    pub pitch_yaw_roll: [f32; 3],
    #[serde(default)]
    pub waypoint_tag: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ParsedMap {
    pub info: MapInfo,
    pub blocks: Vec<MapBlock>,
    pub items: Vec<MapItem>,
    pub header_chunks: Vec<u32>,
    pub body_chunks: Vec<u32>,
    pub warnings: Vec<String>,
    pub source_hash: String,
}

pub fn parse_map(data: &[u8]) -> anyhow::Result<ParsedMap> {
    let mut out = ParsedMap {
        source_hash: blake3::hash(data).to_hex().to_string(),
        ..Default::default()
    };
    let mut r = Reader::new(data);
    if r.bytes(3)? != b"GBX" {
        anyhow::bail!("not a GBX file");
    }
    let version = r.u16()?;
    anyhow::ensure!(version >= 3, "unsupported GBX version {version}");
    let format = r.u8()?;
    anyhow::ensure!(format == b'B', "only binary GBX is supported");
    let _ref_compression = r.u8()?;
    let body_compression = r.u8()?;
    if version >= 4 {
        let _unknown = r.u8()?;
    }
    let class_id = r.u32()?;
    anyhow::ensure!(
        class_id == CLASS_CHALLENGE,
        "not a map (class 0x{class_id:08X})"
    );

    // ---- header chunks
    if version >= 6 {
        let user_data_size = r.u32()? as usize;
        let start = r.pos;
        let n = r.u32()?;
        let mut entries = vec![];
        for _ in 0..n {
            let id = r.u32()?;
            let size = r.u32()? & 0x7FFF_FFFF;
            entries.push((id, size as usize));
        }
        let mut id_state = IdState::default();
        for (id, size) in entries {
            let chunk = r.bytes(size)?;
            out.header_chunks.push(id);
            if let Err(e) = parse_header_chunk(id, chunk, &mut out.info, &mut id_state) {
                out.warnings.push(format!("header chunk 0x{id:08X}: {e}"));
            }
        }
        // be tolerant of size mismatch
        r.pos = start + user_data_size;
    }
    let _num_nodes = r.u32()?;
    // ---- reference table
    let num_external = r.u32()?;
    if num_external > 0 {
        let _ancestor = r.u32()?;
        skip_folders(&mut r)?;
        for _ in 0..num_external {
            let flags = r.u32()?;
            if flags & 4 == 0 {
                let _ = r.string()?;
            } else {
                let _ = r.u32()?;
            }
            let _node_index = r.u32()?;
            if version >= 5 {
                let _use_file = r.u32()?;
            }
            if flags & 4 != 0 {
                let _folder = r.u32()?;
            }
        }
    }
    // ---- body
    let body: Vec<u8> = if body_compression == b'C' {
        let uncompressed = r.u32()? as usize;
        let compressed = r.u32()? as usize;
        let src = r.bytes(compressed)?;
        let mut dst = vec![0u8; uncompressed];
        let n = lzokay::decompress::decompress(src, &mut dst)
            .map_err(|e| anyhow::anyhow!("LZO body decompression failed: {e:?}"))?;
        dst.truncate(n);
        dst
    } else {
        data[r.pos..].to_vec()
    };
    parse_body(&body, &mut out);
    Ok(out)
}

fn skip_folders(r: &mut Reader) -> GbxResult<()> {
    let n = r.u32()?;
    for _ in 0..n {
        let _name = r.string()?;
        skip_folders(r)?;
    }
    Ok(())
}

fn parse_header_chunk(
    id: u32,
    chunk: &[u8],
    info: &mut MapInfo,
    ids: &mut IdState,
) -> GbxResult<()> {
    let mut r = Reader::new(chunk);
    match id {
        0x0304_3002 => {
            let v = r.u8()?;
            if v < 3 {
                let _ = r.ident(ids)?;
                info.name = r.string()?;
            }
            let _ = r.bool()?;
            if v >= 1 {
                info.bronze_ms = r.u32()?;
                info.silver_ms = r.u32()?;
                info.gold_ms = r.u32()?;
                info.author_ms = r.u32()?;
            }
            if v >= 4 {
                let _cost = r.u32()?;
            }
            if v >= 5 {
                info.is_lap_race = r.bool()?;
            }
            if v == 6 {
                let _ = r.bool()?;
            }
            if v >= 7 {
                let _play_mode = r.u32()?;
            }
            if v >= 9 {
                let _author_score = r.u32()?;
            }
            if v >= 10 {
                let _editor = r.u32()?;
            }
            if v >= 11 {
                info.nb_checkpoints = r.u32()?;
                info.nb_laps = r.u32()?;
            }
        }
        0x0304_3003 => {
            let v = r.u8()?;
            let (uid, collection, author) = r.ident(ids)?;
            info.uid = uid;
            info.collection = collection;
            info.author = author;
            info.name = r.string()?;
            let _kind = r.u8()?;
            if v >= 4 {
                let _locked = r.u32()?;
            }
            if v >= 5 {
                let _password = r.string()?;
            }
            if v >= 7 {
                let (d, _, _) = r.ident(ids)?;
                info.decoration = d;
            }
            if v >= 8 {
                let _ = r.vec2()?;
            }
            if v >= 9 {
                let _ = r.vec2()?;
            }
            if v >= 10 {
                r.skip(16)?;
            }
            if v >= 11 {
                info.map_type = r.string()?;
                info.map_style = r.string()?;
            }
            if v >= 12 {
                let _lightmap_cache = r.u64()?;
            }
            if v >= 13 {
                let _lightmap_version = r.u8()?;
            }
            if v >= 14 {
                info.title_id = r.id(ids)?;
            }
        }
        0x0304_3005 => {
            info.xml = r.string()?;
        }
        0x0304_3008 => {
            let _v = r.u32()?;
            let _author_version = r.u32()?;
            info.author = r.string()?;
            info.author_nick = r.string()?;
            let _zone = r.string()?;
            let _extra = r.string()?;
        }
        _ => {}
    }
    Ok(())
}

fn parse_body(body: &[u8], out: &mut ParsedMap) {
    let mut r = Reader::new(body);
    let mut ids = IdState::default();
    let mut nodes = NodeState::default();
    while let Ok(id) = r.u32() {
        if id == FACADE {
            break;
        }
        out.body_chunks.push(id);
        // skippable?
        if r.peek_u32().ok() == Some(SKIP) {
            let _ = r.u32();
            let size = match r.u32() {
                Ok(s) => s as usize,
                Err(e) => {
                    out.warnings.push(format!("chunk 0x{id:08X}: {e}"));
                    break;
                }
            };
            let chunk = match r.bytes(size) {
                Ok(c) => c,
                Err(e) => {
                    out.warnings.push(format!("chunk 0x{id:08X}: {e}"));
                    break;
                }
            };
            match id {
                0x0304_3040 => {
                    let (items, err) = parse_items(chunk);
                    out.items = items;
                    if let Some(e) = err {
                        out.warnings.push(format!(
                            "items chunk 0x03043040 (partial, {} items): {e}",
                            out.items.len()
                        ));
                    }
                }
                0x0304_3018 => {
                    let mut c = Reader::new(chunk);
                    if let (Ok(lap), Ok(n)) = (c.bool(), c.u32()) {
                        out.info.is_lap_race = lap;
                        out.info.nb_laps = n;
                    }
                }
                0x0304_305F => {
                    // free block transforms, in block order
                    let mut c = Reader::new(chunk);
                    let _version = c.u32().unwrap_or(0);
                    for b in out.blocks.iter_mut().filter(|b| b.free) {
                        match (c.vec3(), c.vec3()) {
                            (Ok(pos), Ok(pyr)) => {
                                b.free_pos = Some(pos);
                                b.free_pyr = Some(pyr);
                            }
                            _ => {
                                out.warnings.push(
                                    "free block chunk 0x0304305F shorter than expected".into(),
                                );
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let res = parse_body_chunk(id, &mut r, out, &mut ids, &mut nodes);
        if let Err(e) = res {
            // Recover: scan forward for the next skippable map chunk
            // (id 0x030430xx followed by "PIKS") and resume there.
            match scan_next_skippable(body, r.pos) {
                Some(next) => {
                    out.warnings.push(format!("body chunk 0x{id:08X} at {}: {e}; resumed at next skippable chunk (offset {next})", r.pos));
                    r.pos = next;
                }
                None => {
                    out.warnings.push(format!(
                        "stopped at body chunk 0x{id:08X} (offset {}): {e}",
                        r.pos
                    ));
                    break;
                }
            }
        }
    }
}

fn scan_next_skippable(body: &[u8], from: usize) -> Option<usize> {
    let mut i = from.saturating_sub(4);
    while i + 8 <= body.len() {
        let id = u32::from_le_bytes(body[i..i + 4].try_into().unwrap());
        if id & 0xFFFF_F000 == CLASS_CHALLENGE {
            let next = u32::from_le_bytes(body[i + 4..i + 8].try_into().unwrap());
            if next == SKIP {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn parse_body_chunk(
    id: u32,
    r: &mut Reader,
    out: &mut ParsedMap,
    ids: &mut IdState,
    nodes: &mut NodeState,
) -> GbxResult<()> {
    match id {
        0x0304_300D => {
            let (v, _, _) = r.ident(ids)?;
            out.info.vehicle = v;
        }
        0x0304_3011 => {
            // collector list, challenge parameters, kind
            read_node(r, ids, nodes)?;
            read_node(r, ids, nodes)?;
            let _kind = r.u32()?;
        }
        0x0304_301F => parse_blocks(r, out, ids, nodes)?,
        0x0304_3021 => {
            read_node(r, ids, nodes)?;
            read_node(r, ids, nodes)?;
            read_node(r, ids, nodes)?;
        }
        0x0304_3022 => {
            let _ = r.u32()?;
        }
        0x0304_3024 => {
            let _ = r.pack_desc()?;
        }
        0x0304_3025 => {
            let _ = r.vec2()?;
            let _ = r.vec2()?;
        }
        0x0304_3026 => {
            read_node(r, ids, nodes)?;
        }
        0x0304_3028 => {
            let has = r.bool()?;
            if has {
                let _ = r.u8()?; // archive gm cam val
                let _ = r.vec3()?;
                let _ = r.vec3()?;
                let _ = r.vec3()?;
                let _ = r.vec3()?;
                let _ = r.f32()?;
                let _ = r.f32()?;
                let _ = r.f32()?;
            }
            let _comments = r.string()?;
        }
        0x0304_302A => {
            let _ = r.bool()?;
        }
        0x0304_3049 => {
            let version = r.u32()?;
            let n = if version >= 2 { 5 } else { 4 };
            for _ in 0..n {
                if read_node(r, ids, nodes)?.is_some() {
                    return Err(GbxError::Format("media clips present; not parsed".into()));
                }
            }
            if version >= 1 {
                let _ = r.u32()?;
                let _ = r.u32()?;
                let _ = r.u32()?;
            }
        }
        other => {
            return Err(GbxError::Format(format!(
                "unknown non-skippable chunk 0x{other:08X}"
            )))
        }
    }
    Ok(())
}

/// Read a node reference; returns the class id (or None for null).
fn read_node(r: &mut Reader, ids: &mut IdState, nodes: &mut NodeState) -> GbxResult<Option<u32>> {
    let index = r.u32()?;
    if index == 0xFFFF_FFFF {
        return Ok(None);
    }
    if let Some(c) = nodes.seen.get(&index) {
        return Ok(Some(*c));
    }
    let class = r.u32()?;
    nodes.seen.insert(index, class);
    read_node_body(class, r, ids, nodes)?;
    Ok(Some(class))
}

/// Parsed content of the nodes we care about.
#[derive(Default)]
struct NodeData {
    waypoint_tag: Option<String>,
    waypoint_order: Option<u32>,
    skin: Option<String>,
}

fn read_node_body(
    class: u32,
    r: &mut Reader,
    ids: &mut IdState,
    nodes: &mut NodeState,
) -> GbxResult<NodeData> {
    let mut data = NodeData::default();
    loop {
        let id = r.u32()?;
        if id == FACADE {
            break;
        }
        if r.peek_u32()? == SKIP {
            let _ = r.u32()?;
            let size = r.u32()? as usize;
            let chunk = r.bytes(size)?;
            // skippable chunks we understand
            if class == 0x2E00_9000 && id == 0x2E00_9001 {
                let _ = chunk;
            }
            continue;
        }
        match (class, id) {
            // CGameCtnBlockSkin
            (0x0305_9000, 0x0305_9000) => {
                let _ = r.string()?;
                let _ = r.string()?;
            }
            (0x0305_9000, 0x0305_9001) => {
                let _ = r.string()?;
                data.skin = Some(r.pack_desc()?);
            }
            (0x0305_9000, 0x0305_9002) => {
                let _ = r.string()?;
                data.skin = Some(r.pack_desc()?);
                let _ = r.pack_desc()?;
            }
            (0x0305_9000, 0x0305_9003) => {
                let _ = r.u32()?;
                let _ = r.pack_desc()?;
            }
            // CGameWaypointSpecialProperty
            (0x2E00_9000, 0x2E00_9000) => {
                let v = r.u32()?;
                if v == 1 {
                    let _spawn = r.u32()?;
                    data.waypoint_order = Some(r.u32()?);
                } else {
                    data.waypoint_tag = Some(r.string()?);
                    data.waypoint_order = Some(r.u32()?);
                }
            }
            // CGameCtnCollectorList
            (0x0301_B000, 0x0301_B000) => {
                let n = r.u32()?;
                for _ in 0..n {
                    let _ = r.ident(ids)?;
                    let _ = r.u32()?;
                }
            }
            // CGameCtnChallengeParameters
            (0x0305_B000, 0x0305_B001) => {
                for _ in 0..4 {
                    let _ = r.string()?;
                }
            }
            (0x0305_B000, 0x0305_B004) => {
                for _ in 0..5 {
                    let _ = r.u32()?;
                }
            }
            (0x0305_B000, 0x0305_B008) => {
                let _ = r.u32()?;
                let _ = r.u32()?;
            }
            (0x0305_B000, 0x0305_B00D) => {
                read_node(r, ids, nodes)?;
            }
            _ => {
                return Err(GbxError::Format(format!(
                    "unknown chunk 0x{id:08X} in node class 0x{class:08X}"
                )))
            }
        }
    }
    Ok(data)
}

fn parse_blocks(
    r: &mut Reader,
    out: &mut ParsedMap,
    ids: &mut IdState,
    nodes: &mut NodeState,
) -> GbxResult<()> {
    let (uid, collection, author) = r.ident(ids)?;
    if out.info.uid.is_empty() {
        out.info.uid = uid;
        out.info.collection = collection;
        out.info.author = author;
    }
    let name = r.string()?;
    if out.info.name.is_empty() {
        out.info.name = name;
    }
    let (deco, _, _) = r.ident(ids)?;
    out.info.decoration = deco;
    out.info.size = [r.u32()?, r.u32()?, r.u32()?];
    let _need_unlock = r.bool()?;
    let version = r.u32()?;
    let nb = r.u32()? as usize;
    let mut count = 0usize;
    let mut guard = 0usize;
    while count < nb && guard < nb * 4 + 64 {
        guard += 1;
        let name = r.id(ids)?;
        let dir = r.u8()?;
        let coord = [r.u8()?, r.u8()?, r.u8()?];
        let flags = if version >= 6 {
            r.u32()?
        } else {
            r.u16()? as u32
        };
        if flags == 0xFFFF_FFFF {
            continue;
        }
        let mut block = MapBlock {
            name,
            dir: dir & 3,
            coord,
            flags,
            ground: flags & 0x1000 != 0,
            free: flags & 0x2000_0000 != 0,
            variant: flags & 0x3F,
            waypoint_tag: None,
            waypoint_order: None,
            skin: None,
            free_pos: None,
            free_pyr: None,
        };
        if flags & 0x8000 != 0 {
            let _author = r.id(ids)?;
            let idx = r.u32()?;
            if idx != 0xFFFF_FFFF && !nodes.seen.contains_key(&idx) {
                let class = r.u32()?;
                nodes.seen.insert(idx, class);
                let d = read_node_body(class, r, ids, nodes)?;
                block.skin = d.skin;
            }
        }
        if flags & 0x0010_0000 != 0 {
            let idx = r.u32()?;
            if idx != 0xFFFF_FFFF && !nodes.seen.contains_key(&idx) {
                let class = r.u32()?;
                nodes.seen.insert(idx, class);
                let d = read_node_body(class, r, ids, nodes)?;
                block.waypoint_tag = d.waypoint_tag;
                block.waypoint_order = d.waypoint_order;
            }
        }
        out.blocks.push(block);
        count += 1;
    }
    Ok(())
}

/// Best-effort decode of the anchored-objects chunk (0x03043040). Returns
/// whatever was decoded plus the error that stopped decoding, if any.
fn parse_items(chunk: &[u8]) -> (Vec<MapItem>, Option<GbxError>) {
    let mut items = vec![];
    match parse_items_inner(chunk, &mut items) {
        Ok(()) => (items, None),
        Err(e) => (items, Some(e)),
    }
}

fn parse_items_inner(chunk: &[u8], items: &mut Vec<MapItem>) -> GbxResult<()> {
    let mut r = Reader::new(chunk);
    let version = r.u32()?;
    let _u01 = r.u32()?;
    let _size = r.u32()?;
    let _u02 = r.u32()?;
    let n = r.u32()? as usize;
    let mut ids = IdState::default();
    let mut nodes = NodeState::default();
    for _ in 0..n {
        // items are written inline: class id, then chunks (no node index)
        let class = r.u32()?;
        if class != 0x0310_1000 {
            return Err(GbxError::Format(format!(
                "expected CGameCtnAnchoredObject, got 0x{class:08X}"
            )));
        }
        let mut item = MapItem {
            model: String::new(),
            pos: [0.0; 3],
            pitch_yaw_roll: [0.0; 3],
            waypoint_tag: None,
        };
        loop {
            let cid = r.u32()?;
            if cid == FACADE {
                break;
            }
            if r.peek_u32()? == SKIP {
                let _ = r.u32()?;
                let size = r.u32()? as usize;
                r.skip(size)?;
                continue;
            }
            match cid {
                0x0310_1002 => {
                    let v = r.u32()?;
                    let (model, _, _) = r.ident(&mut ids)?;
                    item.model = model;
                    item.pitch_yaw_roll = r.vec3()?;
                    let _block_coord = [r.u8()?, r.u8()?, r.u8()?];
                    let _anchor_tree = r.id(&mut ids)?;
                    item.pos = r.vec3()?;
                    let wp = r.u32()?;
                    if wp != 0xFFFF_FFFF && !nodes.seen.contains_key(&wp) {
                        let class = r.u32()?;
                        nodes.seen.insert(wp, class);
                        let d = read_node_body(class, &mut r, &mut ids, &mut nodes)?;
                        item.waypoint_tag = d.waypoint_tag;
                    }
                    if v >= 4 {
                        let _flags = r.u16()?;
                    }
                    if v >= 5 {
                        let _pivot = r.vec3()?;
                    }
                    if v >= 6 {
                        let _scale = r.f32()?;
                    }
                    if v >= 7 {
                        let _ = r.u32()?;
                        let _ = r.u32()?;
                        let _ = r.u32()?;
                    }
                    if v >= 8 {
                        let _ = r.vec3()?;
                        let _ = r.vec3()?;
                    }
                }
                other => {
                    return Err(GbxError::Format(format!(
                        "unknown item chunk 0x{other:08X} (items version {version})"
                    )))
                }
            }
        }
        items.push(item);
    }
    Ok(())
}
