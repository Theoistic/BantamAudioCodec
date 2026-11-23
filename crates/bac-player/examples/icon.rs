//! Writes the program's icon: `cargo run -p bac-player --example icon -- bac.ico` (or a `.png` to look at it).

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "bac.ico".into());
    let bytes = if out.ends_with(".png") {
        // over the previewer's background, since the PNG writer keeps no cover channel
        let px: Vec<u32> = bac_player::icon::render(256).iter().map(|&p| bac_player::canvas::blend(0x1A1B1E, p & 0xFF_FFFF, (p >> 24) as f32 / 255.0)).collect();
        bac_player::canvas::png(256, 256, &px)
    } else {
        bac_player::icon::ico()
    };
    std::fs::write(&out, bytes).expect("the icon could not be written");
    println!("{out}");
}
