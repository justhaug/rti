//! Extract the embedded map from a replay to a .Map.Gbx file.
use rti_maps::container::read_container;
use rti_maps::reader::Reader;
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let c = read_container(&std::fs::read(&a[1])?)?;
    let mut r = Reader::new(&c.body);
    let id = r.u32()?;
    anyhow::ensure!(id == 0x0309_3002, "unexpected first chunk 0x{id:08X}");
    let size = r.u32()? as usize;
    std::fs::write(&a[2], r.bytes(size)?)?;
    println!("wrote {} bytes to {}", size, a[2]);
    Ok(())
}
