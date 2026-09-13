//! Exploratory dump of a .Replay.Gbx: header chunks and body chunk structure.
use rti_maps::container::{read_container, scan_next_skippable, FACADE, SKIP};
use rti_maps::reader::Reader;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("replay path");
    let data = std::fs::read(&path)?;
    let c = read_container(&data)?;
    println!(
        "class 0x{:08X} version {} body {} bytes",
        c.class_id,
        c.version,
        c.body.len()
    );
    for (id, b) in &c.header_chunks {
        println!("header 0x{id:08X} {} bytes: {}", b.len(), preview(b));
    }
    let body = &c.body;
    let mut r = Reader::new(body);
    let mut pos_guard = 0;
    while !r.eof() && pos_guard < 200 {
        pos_guard += 1;
        let at = r.pos;
        let id = match r.u32() {
            Ok(v) => v,
            Err(_) => break,
        };
        if id == FACADE {
            println!("  FACADE at {at}");
            break;
        }
        if r.peek_u32().ok() == Some(SKIP) {
            let _ = r.u32();
            let size = r.u32()? as usize;
            let chunk = r.bytes(size)?;
            println!(
                "  chunk 0x{id:08X} skippable {size} bytes at {at}: {}",
                preview(chunk)
            );
            continue;
        }
        println!(
            "  chunk 0x{id:08X} NON-skippable at {at}: next bytes {}",
            preview(&body[r.pos..(r.pos + 64).min(body.len())])
        );
        // known replay chunks we can step over
        match id {
            0x0309_3002 => {
                let size = r.u32()? as usize;
                println!(
                    "     embedded map: {size} bytes (GBX? {})",
                    &body[r.pos..r.pos + 3] == b"GBX"
                );
                r.skip(size)?;
            }
            0x0309_3014 => {
                let version = r.u32()?;
                let count = r.u32()?;
                println!("     ghosts chunk version {version} count {count}; ghost nodes follow (dumping raw)");
                dump_ghost(&mut r, body)?;
                break;
            }
            _ => match scan_next_skippable(body, r.pos, id & 0xFFFF_F000) {
                Some(n) => {
                    println!("     unknown; resuming at {n}");
                    r.pos = n;
                }
                None => break,
            },
        }
    }
    Ok(())
}

fn dump_ghost(r: &mut Reader, body: &[u8]) -> anyhow::Result<()> {
    // node ref: index, class
    let idx = r.u32()?;
    let class = r.u32()?;
    println!("     ghost node idx {idx} class 0x{class:08X}");
    let mut guard = 0;
    while guard < 100 {
        guard += 1;
        let at = r.pos;
        let id = r.u32()?;
        if id == FACADE {
            println!("     ghost FACADE at {at}");
            break;
        }
        if r.peek_u32().ok() == Some(SKIP) {
            let _ = r.u32();
            let size = r.u32()? as usize;
            let chunk = r.bytes(size)?;
            println!(
                "     ghost chunk 0x{id:08X} skippable {size} bytes: {}",
                preview(chunk)
            );
            continue;
        }
        println!(
            "     ghost chunk 0x{id:08X} NON-skippable at {at}: {}",
            preview(&body[r.pos..(r.pos + 48).min(body.len())])
        );
        match id {
            0x0303_F006 => {
                let _ = r.u32()?;
                ghost_data(r)?;
            }
            0x0303_F005 => {
                ghost_data(r)?;
            }
            0x0309_2005 | 0x0309_2008 | 0x0309_200A | 0x0309_200C | 0x0309_2014 => {
                let v = r.u32()?;
                println!("       u32 {v}");
            }
            0x0309_2009 => {
                let v = r.vec3()?;
                println!("       vec3 {v:?}");
            }
            0x0309_200B => {
                let n = r.u32()?;
                let mut v = vec![];
                for _ in 0..n {
                    v.push(r.u64()?);
                }
                println!("       checkpoints {v:?}");
            }
            0x0309_200F => {
                let s = r.string()?;
                println!("       login {s:?}");
            }
            0x0309_200E => {
                let v = r.u32()?;
                println!("       uid id 0x{v:08x}");
            }
            0x0309_2010 | 0x0309_2011 => {
                let v = r.u32()?;
                if v & 0x3fff_ffff == 0 && v & 0xC000_0000 != 0 {
                    let s = r.string()?;
                    println!("       id string {s:?}");
                } else {
                    println!("       id 0x{v:08x}");
                }
            }
            0x0309_2018 => {
                for _ in 0..3 {
                    let v = r.u32()?;
                    if v & 0x3fff_ffff == 0 && v & 0xC000_0000 != 0 {
                        let s = r.string()?;
                        println!("       ident str {s:?}");
                    } else {
                        println!("       ident id 0x{v:08x}");
                    }
                }
            }
            0x0309_201C => {
                for _ in 0..8 {
                    let v = r.u32()?;
                    print!(" {v}");
                }
                println!();
            }
            0x0309_2012 => {
                let a = r.u32()?;
                let b = r.u64()?;
                let c = r.u64()?;
                println!("       {a} {b} {c}");
            }
            _ => {
                println!("       unknown ghost chunk; stopping");
                break;
            }
        }
    }
    Ok(())
}

fn ghost_data(r: &mut Reader) -> anyhow::Result<()> {
    let unc = r.u32()? as usize;
    let comp = r.u32()? as usize;
    let src = r.bytes(comp)?;
    let data = rti_maps::container::zlib_inflate(src, unc)?;
    println!(
        "       ghost data: {unc} bytes uncompressed ({comp} compressed), got {}",
        data.len()
    );
    if data.len() < 16 {
        println!("       (empty legacy ghost data)");
        return Ok(());
    }
    let mut g = Reader::new(&data);
    let class = g.u32()?;
    let skip2 = g.u32()?;
    println!("       classId 0x{class:08X} bSkipList2 {skip2}");
    let u01 = g.u32()?;
    let period = g.u32()?;
    let u02 = g.u32()?;
    println!("       u01 {u01} samplePeriod {period} u02 {u02}");
    let size = g.u32()?;
    println!("       size {size}");
    println!(
        "       next: {}",
        preview(&data[g.pos..(g.pos + 96).min(data.len())])
    );
    std::fs::write("/tmp/claude-1000/-home-justin-dev-rti/99fe4969-2d92-4e3d-b924-87750440e9ea/scratchpad/ghostdata.bin", &data)?;
    Ok(())
}

fn preview(b: &[u8]) -> String {
    let n = b.len().min(48);
    let hex: Vec<String> = b[..n].iter().map(|x| format!("{x:02x}")).collect();
    let asc: String = b[..n]
        .iter()
        .map(|&x| {
            if (32..127).contains(&x) {
                x as char
            } else {
                '.'
            }
        })
        .collect();
    format!("{} |{}|", hex.join(""), asc)
}
