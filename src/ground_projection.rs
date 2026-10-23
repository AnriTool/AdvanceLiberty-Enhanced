//! Offline BG2 reference model and fractional-camera sampling. Never patches guest.
use gba_core::{ppu, Machine, MemMap};
use std::{io::Write, path::Path};

fn word(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn half(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn scaled(v: i32) -> i32 {
    v.wrapping_mul(40) >> 6
}
fn signed28(v: i32) -> i32 {
    v.wrapping_shl(4) >> 4
}

#[derive(Clone, Copy)]
struct Model {
    inputs: [i32; 6],
    x: i32,
    y: i32,
    pa: u16,
    pd: u16,
}
impl Model {
    fn from_inputs(inputs: [i32; 6]) -> Self {
        let [left, right, top, _bottom, sx, sy] = inputs;
        let left = scaled(left).wrapping_sub(8);
        let right = scaled(right).wrapping_add(8);
        let top = scaled(top).wrapping_sub(8);
        // The original uses HALF THE X SPAN for both reference coordinates.
        let mid = right.wrapping_sub(left).wrapping_shl(16) >> 1;
        let x = left
            .wrapping_shl(16)
            .wrapping_add(mid)
            .wrapping_sub(scaled(sx).wrapping_mul(120))
            >> 8;
        let y = top
            .wrapping_shl(16)
            .wrapping_add(mid)
            .wrapping_sub(scaled(sy).wrapping_mul(120))
            >> 8;
        Self {
            inputs,
            x,
            y,
            pa: (sx.wrapping_mul(40) >> 14) as u16,
            pd: (sy.wrapping_mul(40) >> 14) as u16,
        }
    }
}
#[derive(Default)]
pub struct Observer {
    pending: Option<Model>,
    rows: Vec<String>,
}
impl Observer {
    #[inline]
    pub fn interested(key: u32) -> bool {
        matches!(key, 0x080105c5 | 0x08010681)
    }
    pub fn observe(&mut self, m: &Machine, key: u32) -> Result<(), String> {
        if key == 0x080105c5 {
            if self.pending.is_some() {
                return Err("Nested ground calculation".into());
            }
            let inputs = [0x50, 0x58, 0x54, 0x5c, 0x68, 0x6c].map(|o| word(&m.bus.iwram, o));
            self.pending = Some(Model::from_inputs(inputs));
        }
        if key == 0x08010681 {
            let model = self.pending.take().ok_or("Missing ground inputs")?;
            let actual = [
                half(&m.bus.io, 0x20),
                half(&m.bus.io, 0x22),
                half(&m.bus.io, 0x24),
                half(&m.bus.io, 0x26),
            ];
            if actual != [model.pa, 0, 0, model.pd]
                || word(&m.bus.io, 0x28) != model.x
                || word(&m.bus.io, 0x2c) != model.y
            {
                return Err(format!(
                    "Ground model mismatch: actual={actual:?} x={} y={} expected={:?} x={} y={}",
                    word(&m.bus.io, 0x28),
                    word(&m.bus.io, 0x2c),
                    [model.pa, 0, 0, model.pd],
                    model.x,
                    model.y
                ));
            }
            self.rows.push(format!(
                "{},{},{},{},{},{},{},{},{},{},{}",
                m.bus.frames,
                model.inputs[0],
                model.inputs[1],
                model.inputs[2],
                model.inputs[3],
                model.inputs[4],
                model.inputs[5],
                model.x,
                model.y,
                model.pa as i16,
                model.pd as i16
            ));
        }
        Ok(())
    }
    pub fn write(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if self.pending.is_some() || self.rows.is_empty() {
            return Err("Ground model observation incomplete".into());
        }
        let mut f = std::fs::File::create_new(path)?;
        writeln!(
            f,
            "frame,left,right,top,bottom,scale_x,scale_y,bg2x,bg2y,pa,pd"
        )?;
        for row in &self.rows {
            writeln!(f, "{row}")?;
        }
        Ok(())
    }
}

// Camera world units -> texture texels: 5/8. References carry 8 fractional bits.
pub fn delta_reference(raw_delta: i32) -> f64 {
    raw_delta as f64 * 160.0 / 65536.0
}
fn sample(source: &MemMap, x: f64, y: f64, delta_x: f64, delta_y: f64) -> u16 {
    let matrix = |o| half(&source.io, o) as i16 as f64;
    let tx =
        ((signed28(word(&source.io, 0x28)) as f64 + delta_x + matrix(0x20) * x + matrix(0x22) * y)
            / 256.0)
            .floor() as i32;
    let ty =
        ((signed28(word(&source.io, 0x2c)) as f64 + delta_y + matrix(0x24) * x + matrix(0x26) * y)
            / 256.0)
            .floor() as i32;
    texel(source, tx, ty)
}
fn texel(source: &MemMap, tx: i32, ty: i32) -> u16 {
    texel_layer(source, tx, ty).unwrap_or(half(&source.palette, 0) & 0x7fff)
}
fn texel_layer(source: &MemMap, tx: i32, ty: i32) -> Option<u16> {
    let cnt = half(&source.io, 0xc);
    let size = 128i32 << ((cnt >> 14) & 3);
    let (tx, ty) = if cnt & 0x2000 != 0 {
        (tx & (size - 1), ty & (size - 1))
    } else if tx < 0 || ty < 0 || tx >= size || ty >= size {
        return None;
    } else {
        (tx, ty)
    };
    let map = ((cnt as usize >> 8) & 31) * 2048;
    let chr = ((cnt as usize >> 2) & 3) * 16384;
    let tile = source.vram[map + (ty / 8 * (size / 8) + tx / 8) as usize] as usize;
    let addr = chr + tile * 64 + (ty % 8 * 8 + tx % 8) as usize;
    let index = if addr < 0x10000 {
        source.vram[addr] as usize
    } else {
        0
    };
    (index != 0).then(|| half(&source.palette, index * 2) & 0x7fff)
}
// Absolute, unrounded world->texture transform. It does not re-anchor to a
// differently rounded hardware snapshot on each scene update.
fn continuous_parameters(source: &MemMap) -> [f64; 4] {
    let cx = delta_reference(word(&source.iwram, 0x14));
    let cy = delta_reference(word(&source.iwram, 0x18));
    let pa = delta_reference(word(&source.iwram, 0x68));
    let pd = delta_reference(word(&source.iwram, 0x6c));
    [cx - 120.0 * pa, cy - 120.0 * pd, pa, pd]
}
fn continuous(source: &MemMap, x: f64, y: f64, dx: f64, dy: f64) -> u16 {
    let [rx, ry, pa, pd] = continuous_parameters(source);
    texel(
        source,
        ((rx + dx + pa * x) / 256.0).floor() as i32,
        ((ry + dy + pd * y) / 256.0).floor() as i32,
    )
}
type Sampler = fn(&MemMap, f64, f64, f64, f64) -> u16;
fn reference(source: &MemMap) -> Vec<u16> {
    let mut gpu = MemMap::new(Vec::new());
    gpu.io.clone_from(&source.io);
    gpu.vram.clone_from(&source.vram);
    gpu.palette.clone_from(&source.palette);
    // Isolate raw BG2, including transparent texels over the backdrop.
    gpu.io[..2].copy_from_slice(&0x0401u16.to_le_bytes());
    gpu.io[0x50..0x56].fill(0);
    gpu.aff_dirty = [true; 2];
    for y in 0..160 {
        ppu::render_scanline(&mut gpu, y);
    }
    gpu.framebuffer
}
fn save_image(
    path: &Path,
    source: &MemMap,
    scale: usize,
    delta_x: f64,
    delta_y: f64,
    sampler: Sampler,
) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create_new(path)?);
    writeln!(f, "P6\n{} {}\n255", 240 * scale, 160 * scale)?;
    for y in 0..160 * scale {
        for x in 0..240 * scale {
            let p = sampler(
                source,
                x as f64 / scale as f64,
                y as f64 / scale as f64,
                delta_x,
                delta_y,
            );
            for shift in [0, 5, 10] {
                let c = ((p >> shift) & 31) as u8;
                f.write_all(&[(c << 3) | (c >> 2)])?;
            }
        }
    }
    f.flush()
}
pub fn continuous_image(source: &MemMap, scale: usize, dx: i32, dy: i32) -> Vec<u16> {
    (0..160 * scale)
        .flat_map(|y| {
            (0..240 * scale).map(move |x| {
                continuous(
                    source,
                    x as f64 / scale as f64,
                    y as f64 / scale as f64,
                    delta_reference(dx),
                    delta_reference(dy),
                )
            })
        })
        .collect()
}
pub fn continuous_layer(source: &MemMap, scale: usize, delta: [i32; 2]) -> Vec<Option<u16>> {
    continuous_layer_zoom(source, scale, delta, 1.0)
}
pub fn continuous_layer_zoom(
    source: &MemMap,
    scale: usize,
    delta: [i32; 2],
    zoom: f64,
) -> Vec<Option<u16>> {
    let [rx, ry, pa, pd] = continuous_parameters(source);
    let (dx, dy) = (delta_reference(delta[0]), delta_reference(delta[1]));
    let width = 240 * scale;
    let xs: Vec<_> = (0..width)
        .map(|x| {
            let sx = x as f64 / scale as f64;
            let sx = if zoom == 1.0 {
                sx
            } else {
                120.0 + (sx - 120.0) / zoom
            };
            ((rx + dx + pa * sx) / 256.0).floor() as i32
        })
        .collect();
    let mut layer = vec![None; width * 160 * scale];
    let mut previous_y = None;
    for y in 0..160 * scale {
        let sy = y as f64 / scale as f64;
        let sy = if zoom == 1.0 {
            sy
        } else {
            120.0 + (sy - 120.0) / zoom
        };
        let ty = ((ry + dy + pd * sy) / 256.0).floor() as i32;
        if previous_y == Some(ty) {
            layer.copy_within((y - 1) * width..y * width, y * width);
        } else {
            let mut previous_x = None;
            let mut pixel = None;
            for (x, &tx) in xs.iter().enumerate() {
                if previous_x != Some(tx) {
                    pixel = texel_layer(source, tx, ty);
                    previous_x = Some(tx);
                }
                layer[y * width + x] = pixel;
            }
        }
        previous_y = Some(ty);
    }
    layer
}
pub fn validate_live(source: &MemMap) -> Result<(), String> {
    let cnt = half(&source.io, 0);
    if cnt & 7 != 1 || cnt & 0x400 == 0 || cnt & 0x80 != 0 || half(&source.io, 0xc) & 0x40 != 0 {
        return Err("Unsupported ground mode".into());
    }
    let cx = word(&source.iwram, 0x14) >> 16;
    let cy = word(&source.iwram, 0x18) >> 16;
    let left = word(&source.iwram, 0x50);
    let right = word(&source.iwram, 0x58);
    if left + right != cx * 2
        || word(&source.iwram, 0x54) != cy - (right - left) / 2
        || half(&source.io, 0x22) != 0
        || half(&source.io, 0x24) != 0
    {
        return Err("Unsupported camera bounds".into());
    }
    Ok(())
}
pub fn export(source: &MemMap, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let control = half(&source.io, 0);
    if control & 7 != 1
        || control & 0x400 == 0
        || control & 0x80 != 0
        || half(&source.io, 0xc) & 0x40 != 0
    {
        return Err(
            "Ground probe requires enabled affine BG2 mode 1 without forced blank or mosaic".into(),
        );
    }
    let cx = word(&source.iwram, 0x14) >> 16;
    let cy = word(&source.iwram, 0x18) >> 16;
    let left = word(&source.iwram, 0x50);
    let right = word(&source.iwram, 0x58);
    if left + right != cx * 2
        || word(&source.iwram, 0x54) != cy - (right - left) / 2
        || half(&source.io, 0x22) != 0
        || half(&source.io, 0x24) != 0
    {
        return Err(
            "Ground continuous model requires symmetric USA camera bounds and axis-aligned BG2"
                .into(),
        );
    }
    let pixels = reference(source);
    for y in 0..160 {
        for x in 0..240 {
            if sample(source, x as f64, y as f64, 0.0, 0.0) != pixels[y * 240 + x] {
                return Err(format!("Ground sampler mismatch at {x},{y}").into());
            }
        }
    }
    std::fs::create_dir(path)?;
    let mut report = std::fs::File::create_new(path.join("samples.csv"))?;
    writeln!(
        report,
        "model,axis,delta_raw,delta_reference_8,changed_pixels_3x"
    )?;
    for (name, sampler) in [
        ("anchored", sample as Sampler),
        ("continuous", continuous as Sampler),
    ] {
        save_image(
            &path.join(format!("{name}-zero.ppm")),
            source,
            3,
            0.0,
            0.0,
            sampler,
        )?;
        for axis in ['x', 'y'] {
            for delta in [16384, 32768, 49152, 65536] {
                let shift = delta_reference(delta);
                let (dx, dy) = if axis == 'x' {
                    (shift, 0.0)
                } else {
                    (0.0, shift)
                };
                let mut changed = 0usize;
                for y in 0..480 {
                    for x in 0..720 {
                        if sampler(source, x as f64 / 3.0, y as f64 / 3.0, dx, dy)
                            != sampler(source, x as f64 / 3.0, y as f64 / 3.0, 0.0, 0.0)
                        {
                            changed += 1;
                        }
                    }
                }
                save_image(
                    &path.join(format!("{name}-{axis}-{delta}.ppm")),
                    source,
                    3,
                    dx,
                    dy,
                    sampler,
                )?;
                writeln!(report, "{name},{axis},{delta},{shift},{changed}")?;
            }
        }
    }
    let mut params = std::fs::File::create_new(path.join("parameters.csv"))?;
    writeln!(params, "model,bg2x_8,bg2y_8,pa_8,pd_8")?;
    writeln!(
        params,
        "hardware,{},{},{},{}",
        word(&source.io, 0x28),
        word(&source.io, 0x2c),
        half(&source.io, 0x20) as i16,
        half(&source.io, 0x26) as i16
    )?;
    let [rx, ry, pa, pd] = continuous_parameters(source);
    writeln!(params, "continuous,{rx},{ry},{pa},{pd}")?;
    std::fs::write(path.join("source.io"), &source.io)?;
    std::fs::write(path.join("source.vram"), &source.vram)?;
    std::fs::write(path.join("source.palette"), &source.palette)?;
    std::fs::write(path.join("source.iwram"), &source.iwram)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zoomed_ground_matches_scalar_world_sampling_with_camera_motion() {
        let mut source = MemMap::new(Vec::new());
        source.io[0xc..0xe].copy_from_slice(&0x2080u16.to_le_bytes());
        for (i, b) in source.vram.iter_mut().enumerate() {
            *b = (i.wrapping_mul(71) % 256) as u8;
        }
        for (i, p) in source.palette.chunks_exact_mut(2).enumerate() {
            p.copy_from_slice(&(i as u16 * 97).to_le_bytes());
        }
        for (o, v) in [
            (0x14, 1000 * 65536i32),
            (0x18, 2000 * 65536),
            (0x68, 89544),
            (0x6c, 89544),
        ] {
            source.iwram[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        for zoom in [0.95, 1.0, 1.05] {
            let scale = 3;
            let delta = [32768, -16384];
            let actual = continuous_layer_zoom(&source, scale, delta, zoom);
            let [rx, ry, pa, pd] = continuous_parameters(&source);
            for y in 0..480 {
                for x in 0..720 {
                    let sx = 120.0 + (x as f64 / 3.0 - 120.0) / zoom;
                    let sy = 120.0 + (y as f64 / 3.0 - 120.0) / zoom;
                    let expected = texel_layer(
                        &source,
                        ((rx + delta_reference(delta[0]) + pa * sx) / 256.0).floor() as i32,
                        ((ry + delta_reference(delta[1]) + pd * sy) / 256.0).floor() as i32,
                    );
                    assert_eq!(actual[y * 720 + x], expected);
                }
            }
        }
    }
    #[test]
    fn repeated_ground_texels_match_scalar_sampler() {
        let mut source = MemMap::new(Vec::new());
        source.io[0xc..0xe].copy_from_slice(&0x2080u16.to_le_bytes());
        for (i, b) in source.vram.iter_mut().enumerate() {
            *b = (i.wrapping_mul(71) % 256) as u8;
        }
        for (i, p) in source.palette.chunks_exact_mut(2).enumerate() {
            p.copy_from_slice(&(i as u16 * 97).to_le_bytes());
        }
        for scale in [1, 3] {
            for zoom in [32768i32, 65536, 131072, -65536] {
                for (offset, value) in [
                    (0x14, -17632123i32),
                    (0x18, 8700011),
                    (0x68, zoom),
                    (0x6c, zoom),
                ] {
                    source.iwram[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                }
                let delta = [-31767, 63111];
                let [rx, ry, pa, pd] = continuous_parameters(&source);
                let (dx, dy) = (delta_reference(delta[0]), delta_reference(delta[1]));
                let expected: Vec<_> = (0..160 * scale)
                    .flat_map(|y| {
                        let source = &source;
                        (0..240 * scale).map(move |x| {
                            texel_layer(
                                source,
                                ((rx + dx + pa * x as f64 / scale as f64) / 256.0).floor() as i32,
                                ((ry + dy + pd * y as f64 / scale as f64) / 256.0).floor() as i32,
                            )
                        })
                    })
                    .collect();
                assert_eq!(continuous_layer(&source, scale, delta), expected);
            }
        }
    }
    #[test]
    fn camera_quarters_retain_fraction_and_negative_direction() {
        assert_eq!(delta_reference(16384), 40.0);
        assert_eq!(delta_reference(32768), 80.0);
        assert_eq!(delta_reference(-16384), -40.0);
        assert_eq!(delta_reference(1), 160.0 / 65536.0);
    }
    #[test]
    fn continuous_model_agrees_when_integer_path_has_no_rounding_loss() {
        for x in [-1024i32, 0, 1024] {
            for y in [-512i32, 512] {
                for scale in [65536i32, 131072] {
                    let model =
                        Model::from_inputs([x - 128, x + 128, y - 128, y + 128, scale, scale]);
                    let mut m = MemMap::new(Vec::new());
                    for (o, v) in [
                        (0x14, x << 16),
                        (0x18, y << 16),
                        (0x68, scale),
                        (0x6c, scale),
                    ] {
                        m.iwram[o..o + 4].copy_from_slice(&v.to_le_bytes());
                    }
                    assert_eq!(
                        continuous_parameters(&m),
                        [
                            model.x as f64,
                            model.y as f64,
                            model.pa as i16 as f64,
                            model.pd as i16 as f64
                        ]
                    );
                }
            }
        }
    }
    #[test]
    fn sampler_matches_ppu_with_wrap_negative_references_and_all_map_sizes() {
        for size in 0..4 {
            for wrap in [false, true] {
                for start in [-257i32, 0, 90000] {
                    let mut m = MemMap::new(Vec::new());
                    let cnt = (size << 14) | if wrap { 0x2000 } else { 0 };
                    m.io[0xc..0xe].copy_from_slice(&(cnt as u16).to_le_bytes());
                    for (i, p) in m.palette.chunks_exact_mut(2).enumerate() {
                        p.copy_from_slice(&((i as u16 * 79) | 0x8000).to_le_bytes());
                    }
                    for (i, b) in m.vram.iter_mut().enumerate() {
                        *b = ((i * 13 + i / 128) % 256) as u8;
                    }
                    for (o, v) in [(0x20, 231i16), (0x22, -21), (0x24, 17), (0x26, 199)] {
                        m.io[o..o + 2].copy_from_slice(&v.to_le_bytes());
                    }
                    m.io[0x28..0x2c].copy_from_slice(&start.to_le_bytes());
                    m.io[0x2c..0x30].copy_from_slice(&(-start).to_le_bytes());
                    let expected = reference(&m);
                    for y in 0..160 {
                        for x in 0..240 {
                            assert_eq!(
                                sample(&m, x as f64, y as f64, 0.0, 0.0),
                                expected[y * 240 + x],
                                "size={size} wrap={wrap} start={start} pixel={x},{y}"
                            );
                        }
                    }
                }
            }
        }
    }
}
