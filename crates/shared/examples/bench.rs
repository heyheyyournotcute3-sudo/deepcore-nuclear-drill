fn main() {
    let content = deepcore_shared::Content::load_from_dir("assets/data").unwrap();
    let generator = deepcore_shared::worldgen::WorldGen::new(&content, deepcore_shared::worldgen::WorldGenConfig::new(7));
    let start = std::time::Instant::now();
    let mut n = 0;
    for y in 6..12 {
        for x in 12..18 {
            for z in 12..18 {
                let _ = generator.generate(deepcore_shared::ChunkPos::new(x, y, z));
                n += 1;
            }
        }
    }
    let elapsed = start.elapsed();
    println!("{n} chunks in {elapsed:?} = {:?} per chunk", elapsed / n);
}
