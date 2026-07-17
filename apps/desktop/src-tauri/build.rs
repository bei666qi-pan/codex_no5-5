use std::fs;
use std::io::BufWriter;
use std::path::Path;

fn main() {
    generate_development_icons();
    tauri_build::build()
}

fn generate_development_icons() {
    const SIZE: u32 = 64;
    let directory = Path::new("icons");
    let png_path = directory.join("icon.png");
    let ico_path = directory.join("icon.ico");
    if png_path.exists() && ico_path.exists() {
        return;
    }
    fs::create_dir_all(directory).expect("create icon directory");

    let pixels = icon_pixels(SIZE);
    if !png_path.exists() {
        write_png(&png_path, SIZE, &pixels);
    }
    if !ico_path.exists() {
        write_ico(&ico_path, SIZE, pixels);
    }
}

fn write_png(path: &Path, size: u32, pixels: &[u8]) {
    let file = fs::File::create(path).expect("create development PNG icon");
    let mut encoder = png::Encoder::new(BufWriter::new(file), size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write icon header");
    writer.write_image_data(pixels).expect("write icon pixels");
}

fn write_ico(path: &Path, size: u32, pixels: Vec<u8>) {
    let image = ico::IconImage::from_rgba_data(size, size, pixels);
    let mut icon = ico::IconDir::new(ico::ResourceType::Icon);
    icon.add_entry(ico::IconDirEntry::encode(&image).expect("encode ICO icon"));
    let file = fs::File::create(path).expect("create development ICO icon");
    icon.write(BufWriter::new(file)).expect("write ICO icon");
}

fn icon_pixels(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
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
    pixels
}
