//! Extract the player-inputs chunk (0x0309201D) from a replay and print it as words/bits.
use rti_maps::container::{read_container, SKIP};
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("replay");
    let c = read_container(&std::fs::read(&path)?)?;
    let b = &c.body;
    let mut i = 0;
    while i + 12 <= b.len() {
        let id = u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        if id == 0x0309_201D && u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) == SKIP {
            let size = u32::from_le_bytes(b[i + 8..i + 12].try_into().unwrap()) as usize;
            let chunk = &b[i + 12..i + 12 + size];
            std::fs::write("/tmp/claude-1000/-home-justin-dev-rti/99fe4969-2d92-4e3d-b924-87750440e9ea/scratchpad/inputs.bin", chunk)?;
            println!("chunk 0x0309201D: {size} bytes");
            let words: Vec<u32> = chunk
                .chunks(4)
                .take(12)
                .map(|w| {
                    u32::from_le_bytes([
                        w[0],
                        w[1],
                        w.get(2).copied().unwrap_or(0),
                        w.get(3).copied().unwrap_or(0),
                    ])
                })
                .collect();
            println!("head words: {:?}", words);
            let data = &chunk[32..];
            println!("data {} bytes; first 96 bytes:", data.len());
            for row in data[..96].chunks(16) {
                println!(
                    "  {}",
                    row.iter()
                        .map(|x| format!("{x:02x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
            println!("as bits (first 40 bytes):");
            for row in data[..40].chunks(8) {
                println!(
                    "  {}",
                    row.iter()
                        .map(|x| format!("{x:08b}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
            // byte histogram
            let mut h = [0usize; 256];
            for &x in data {
                h[x as usize] += 1;
            }
            let mut top: Vec<(usize, usize)> = h
                .iter()
                .enumerate()
                .map(|(i, &c)| (c, i))
                .filter(|(c, _)| *c > 0)
                .map(|(c, i)| (i, c))
                .collect();
            top.sort_by(|a, b| b.1.cmp(&a.1));
            println!("top bytes: {:?}", &top[..top.len().min(12)]);
            return Ok(());
        }
        i += 1;
    }
    println!("not found");
    Ok(())
}
