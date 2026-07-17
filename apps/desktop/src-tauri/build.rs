use std::fs;
use std::io::BufWriter;
use std::path::Path;

fn main() {
    generate_development_icon();
    tauri_build::build()
}

fn generate_development_icon() {
    const SIZE: u32 = 64;
    let directory = Path::new("icons");
    let path = directory.join("icon.png");
    if path.exists() {
        return;
    }
    fs::create_dir_all(directory).expect("create icon directory");
    let file = fs::File::create(path).expect("create development icon");
    let mut encoder = png::Encoder::new(BufWriter::new(file), SIZE, SIZE);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write icon header");
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as i32 - 32;
            let top = (8..=48).contains(&y);
            let shield = top && dx.abs() <= (26 - ((y.saturating_sub(8) / 3) as i32));
            let route = shield && ((28..=35).contains(&x) || (27..=34).contains(&y));
            let rgba = if route {
                [236, 253, 245, 255]
            } else if shield {
                [16, 185, 129, 255]
            } else {
                [15, 23, 42, 255]
            };
            pixels.extend_from_slice(&rgba);
        }
    }
    writer.write_image_data(&pixels).expect("write icon pixels");
}
