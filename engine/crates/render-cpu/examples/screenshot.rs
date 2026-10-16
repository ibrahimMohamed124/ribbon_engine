#[path = "shared/mod.rs"]
mod shared;

use std::fs;
use std::path::PathBuf;

use shared::{graded_style, Scene, SceneShader, HEIGHT, WIDTH};
use voxelworks_core::gfx::Framebuffer;
use voxelworks_render_cpu::render::{render, RenderStyle};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from("target").join("shots");
    fs::create_dir_all(&output)?;
    let scene = Scene::seeded(0x5eed_cafe);
    let shader = SceneShader::default();

    for viewpoint in scene.viewpoints() {
        for (label, style) in [("base", RenderStyle::default()), ("graded", graded_style())] {
            let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT);
            let frame = scene.frame(&viewpoint, style, 1);
            render(&mut framebuffer, &frame, &shader);
            let path = output.join(format!("{}-{label}.png", viewpoint.name));
            write_png(&path, &framebuffer)?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}

fn write_png(path: &std::path::Path, framebuffer: &Framebuffer) -> std::io::Result<()> {
    let mut raw = Vec::with_capacity(framebuffer.height * (1 + framebuffer.width * 3));
    for row in framebuffer.pixels.chunks(framebuffer.width) {
        raw.push(0); // PNG filter: None
        for &pixel in row {
            raw.push((pixel >> 16) as u8);
            raw.push((pixel >> 8) as u8);
            raw.push(pixel as u8);
        }
    }

    let mut zlib = vec![0x78, 0x01]; // zlib header, fastest/no compression
    let mut offset = 0;
    while offset < raw.len() {
        let len = (raw.len() - offset).min(u16::MAX as usize);
        let final_block = offset + len == raw.len();
        zlib.push(u8::from(final_block)); // stored DEFLATE block, byte aligned
        zlib.extend_from_slice(&(len as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(len as u16)).to_le_bytes());
        zlib.extend_from_slice(&raw[offset..offset + len]);
        offset += len;
    }
    let mut adler_a = 1u32;
    let mut adler_b = 0u32;
    for byte in &raw {
        adler_a = (adler_a + u32::from(*byte)) % 65521;
        adler_b = (adler_b + adler_a) % 65521;
    }
    zlib.extend_from_slice(&((adler_b << 16) | adler_a).to_be_bytes());

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(framebuffer.width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(framebuffer.height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    chunk(&mut png, *b"IHDR", &ihdr);
    chunk(&mut png, *b"IDAT", &zlib);
    chunk(&mut png, *b"IEND", &[]);
    fs::write(path, png)
}

fn chunk(png: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(&kind);
    png.extend_from_slice(data);
    let crc = crc32(kind.iter().chain(data.iter()).copied());
    png.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: impl IntoIterator<Item = u8>) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
