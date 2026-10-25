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

// USA terrain: 256x256 (Portland) or 128x256 map cells; each cell references a 5x5 grid of
// 8x8 indexed tiles. The GPU's small resident cache omits offscreen tiles.
#[derive(Clone)]
pub struct WorldGround {
    rom: std::sync::Arc<Vec<u8>>,
    map: usize,
    blocks: usize,
    atlas: usize,
    shift: usize,
    rows: usize,
}
impl WorldGround {
    pub fn capture(source: &MemMap, rom: std::sync::Arc<Vec<u8>>) -> Option<Self> {
        if rom.get(0xac..0xb0)? != b"BGTE" {
            return None;
        }
        let pointer = |o: usize| -> Option<usize> {
            let p = u32::from_le_bytes(source.ewram.get(o..o + 4)?.try_into().ok()?);
            if p >> 24 != 8 {
                return None;
            }
            Some((p - 0x08000000) as usize)
        };
        let shift =
            u32::from_le_bytes(source.ewram.get(0x1ce74..0x1ce78)?.try_into().ok()?) as usize;
        if !matches!(shift, 7 | 8) {
            return None;
        }
        let map = pointer(0x1ce60)?;
        let blocks = pointer(0x1ce64)?;
        let atlas = pointer(0x1ce68)?;
        let read = |o| Some(u32::from_le_bytes(rom.get(o..o + 4)?.try_into().ok()?));
        // The island headers contain independent width/height in world units.
        // Match the live resource pointers, rather than the current mission ID:
        // loading can change that ID before the old scene has been retired.
        let descriptor = [0xcabe98, 0xcabed4, 0xcabf10].into_iter().find(|&o| {
            [map, blocks, atlas]
                .into_iter()
                .enumerate()
                .all(|(i, p)| read(o + 12 + i * 4) == Some(p as u32 + 0x08000000))
        })?;
        let width = read(descriptor + 4)? as usize;
        let height = read(descriptor + 8)? as usize;
        if width % 64 != 0
            || height % 64 != 0
            || width / 64 != 1 << shift
            || height == 0
            || height / 64 > 256
        {
            return None;
        }
        let rows = height / 64;
        let result = Self {
            rom,
            map,
            blocks,
            atlas,
            shift,
            rows,
        };
        result
            .rom
            .get(result.map..result.map + (1 << shift) * rows * 2)?;
        result.rom.get(result.blocks..result.blocks + 50)?;
        result.rom.get(result.atlas..result.atlas + 64)?;
        Some(result)
    }
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        let (tx, ty) = (x / 8, y / 8);
        let cells = 1 << self.shift;
        if tx / 5 >= cells || ty / 5 >= self.rows {
            return None;
        }
        let read = |o: usize| -> Option<usize> {
            Some(u16::from_le_bytes(self.rom.get(o..o + 2)?.try_into().ok()?) as usize)
        };
        let block = read(self.map + ((ty / 5) * cells + tx / 5) * 2)?;
        let tile = read(self.blocks + block * 50 + ((ty % 5) * 5 + tx % 5) * 2)?;
        Some(*self.rom.get(self.atlas + tile * 64 + (y % 8) * 8 + x % 8)? as usize)
    }
    pub fn pixel(&self, source: &MemMap, x: i32, y: i32) -> Option<u16> {
        let index = self.index(x, y)?;
        (index != 0).then(|| half(&source.palette, index * 2) & 0x7fff)
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
#[cfg(test)]
pub fn continuous_layer(source: &MemMap, scale: usize, delta: [i32; 2]) -> Vec<Option<u16>> {
    continuous_layer_zoom(source, scale, delta, 1.0)
}
#[cfg(test)]
pub fn continuous_layer_zoom(
    source: &MemMap,
    scale: usize,
    delta: [i32; 2],
    zoom: f64,
) -> Vec<Option<u16>> {
    continuous_layer_view(
        source,
        crate::viewport::View::new(scale, Default::default()),
        delta,
        zoom,
    )
}
#[cfg(test)]
pub fn continuous_layer_view(
    source: &MemMap,
    view: crate::viewport::View,
    delta: [i32; 2],
    zoom: f64,
) -> Vec<Option<u16>> {
    continuous_layer_world(source, view, delta, zoom, None)
}
pub fn continuous_layer_world(
    source: &MemMap,
    view: crate::viewport::View,
    delta: [i32; 2],
    zoom: f64,
    world: Option<&WorldGround>,
) -> Vec<Option<u16>> {
    let scale = view.scale;
    let [rx, ry, pa, pd] = continuous_parameters(source);
    let (dx, dy) = (delta_reference(delta[0]), delta_reference(delta[1]));
    let width = view.width;
    let xs: Vec<_> = (0..width)
        .map(|x| {
            let sx = view.source_x(x);
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
                    // The resident GBA cache only covers the guest camera. During
                    // host camera motion/zoom its stale tiles can enter even the
                    // central viewport. Use the same world map across the frame;
                    // transparent/out-of-map samples must not expose that cache.
                    pixel = match world {
                        Some(world) => world.pixel(source, tx, ty),
                        None => texel_layer(source, tx, ty),
                    };
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
    #[test]
    fn island_ground_uses_each_map_width_and_checks_its_boundary() {
        for shift in [7, 8] {
            let mut rom = vec![0; 0xcabf4c];
            rom[0xac..0xb0].copy_from_slice(b"BGTE");
            let cells = 1usize << shift;
            for (o, v) in [
                (4, (cells * 64) as u32),
                (8, 256 * 64),
                (12, 0x08001000),
                (16, 0x08021000),
                (20, 0x08022000),
            ] {
                rom[0xcabe98 + o..0xcabe98 + o + 4].copy_from_slice(&v.to_le_bytes());
            }
            // Cell (1,1) must be read with this island's row stride.
            rom[0x1000 + (cells + 1) * 2..0x1000 + (cells + 1) * 2 + 2]
                .copy_from_slice(&1u16.to_le_bytes());
            rom[0x1000 + (200 * cells + 1) * 2..0x1000 + (200 * cells + 1) * 2 + 2]
                .copy_from_slice(&1u16.to_le_bytes());
            for i in 0..25 {
                rom[0x21000 + 50 + i * 2..0x21000 + 52 + i * 2]
                    .copy_from_slice(&1u16.to_le_bytes());
            }
            rom[0x22040..0x22080].fill(1);
            let mut source = MemMap::new(Vec::new());
            for (o, v) in [
                (0x1ce60, 0x08001000u32),
                (0x1ce64, 0x08021000),
                (0x1ce68, 0x08022000),
                (0x1ce74, shift),
            ] {
                source.ewram[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
            source.palette[2..4].copy_from_slice(&0x03e0u16.to_le_bytes());
            let world = WorldGround::capture(&source, std::sync::Arc::new(rom)).unwrap();
            assert_eq!(world.pixel(&source, 40, 40), Some(0x03e0));
            assert_eq!(world.pixel(&source, 40, 8000), Some(0x03e0));
            assert_eq!(world.pixel(&source, 40, 10240), None);
            assert_eq!(world.pixel(&source, 0, 0), None);
            assert_eq!(world.pixel(&source, (cells * 40) as i32, 40), None);
            assert_eq!(world.pixel(&source, -1, 40), None);
        }
    }
    #[test]
    #[ignore = "requires USA ROM and captured three-island session"]
    fn island_ground_matches_resident_cache_in_all_three_regions() {
        let directory = std::path::PathBuf::from(std::env::var_os("GTA_ISLAND_TEST_DIR").unwrap())
            .join("capture");
        let rom = std::sync::Arc::new(std::fs::read("roms/usa.gba").unwrap());
        let mut regions = std::collections::BTreeSet::new();
        let mut checked = 0;
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "ewram") {
                continue;
            }
            let mut source = MemMap::new(Vec::new());
            for (ext, target) in [
                ("ewram", &mut source.ewram),
                ("io", &mut source.io),
                ("iwram", &mut source.iwram),
                ("vram", &mut source.vram),
                ("palette", &mut source.palette),
            ] {
                target.copy_from_slice(&std::fs::read(path.with_extension(ext)).unwrap());
            }
            if half(&source.io, 0) & 0x487 != 0x401 {
                continue;
            }
            let world = WorldGround::capture(&source, rom.clone()).unwrap();
            let region = u32::from_le_bytes(source.ewram[0x303d0..0x303d4].try_into().unwrap());
            if region >= 3 {
                continue;
            }
            regions.insert(region);
            let [rx, ry, pa, pd] = continuous_parameters(&source);
            for y in (16..144).step_by(3) {
                for x in (16..224).step_by(3) {
                    let tx = ((rx + pa * x as f64) / 256.0).floor() as i32;
                    let ty = ((ry + pd * y as f64) / 256.0).floor() as i32;
                    if let Some(resident) = texel_layer(&source, tx, ty) {
                        assert_eq!(
                            world.pixel(&source, tx, ty),
                            Some(resident),
                            "{} region={region} {tx},{ty}",
                            path.display()
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(regions, std::collections::BTreeSet::from([0, 1, 2]));
        assert!(checked > 10000);
        println!("island_ground verified {checked} samples across {regions:?}");
    }
    use super::*;
    #[test]
    fn moving_camera_uses_world_terrain_throughout_viewport_without_stale_cache() {
        let mut source = MemMap::new(Vec::new());
        // Resident tile data belongs to a different part of the map.
        source.io[0xc..0xe].copy_from_slice(&0x2080u16.to_le_bytes());
        source.vram[..64].fill(9);
        source.palette[18..20].copy_from_slice(&0x7c00u16.to_le_bytes());
        source.palette[2..4].copy_from_slice(&0x03e0u16.to_le_bytes());
        for (o, v) in [
            (0x14, 300 * 65536i32),
            (0x18, 300 * 65536),
            (0x68, 104858),
            (0x6c, 104858),
        ] {
            source.iwram[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        let mut rom = vec![0; 0x20000 + 50 + 64];
        // Every world tile is green, with one transparent column.
        for y in 0..8 {
            for x in 1..8 {
                rom[0x20000 + 50 + y * 8 + x] = 1;
            }
        }
        let world = WorldGround {
            rom: std::sync::Arc::new(rom),
            map: 0,
            blocks: 0x20000,
            atlas: 0x20000 + 50,
            shift: 8,
            rows: 256,
        };
        for aspect in [
            crate::viewport::Aspect::Original,
            crate::viewport::Aspect::Wide,
        ] {
            let view = crate::viewport::View::new(3, aspect);
            for zoom in [0.95, 1.0, 1.05] {
                let delta = [32768, -16384];
                let layer = continuous_layer_world(&source, view, delta, zoom, Some(&world));
                let [rx, ry, pa, pd] = continuous_parameters(&source);
                let mut transparent = 0;
                for y in 0..view.height {
                    for x in 0..view.width {
                        let sx = 120.0 + (view.source_x(x) - 120.0) / zoom;
                        let sy = 120.0 + (y as f64 / 3.0 - 120.0) / zoom;
                        let tx =
                            ((rx + delta_reference(delta[0]) + pa * sx) / 256.0).floor() as i32;
                        let ty =
                            ((ry + delta_reference(delta[1]) + pd * sy) / 256.0).floor() as i32;
                        let expected = if tx < 0 || ty < 0 || tx % 8 == 0 {
                            None
                        } else {
                            Some(0x03e0)
                        };
                        assert_eq!(layer[y * view.width + x], expected, "{x},{y} zoom={zoom}");
                        transparent += usize::from(expected.is_none());
                    }
                }
                assert!(transparent > 0);
                assert!(layer.iter().any(Option::is_some));
            }
        }
    }
    #[test]
    #[ignore = "local ROM and captured scene cross-check; ADVANCE_WORLD_ROM and ADVANCE_HUD_SCENES"]
    fn world_ground_matches_resident_tiles_in_captured_gameplay() {
        let rom = std::sync::Arc::new(
            std::fs::read(std::env::var("ADVANCE_WORLD_ROM").unwrap()).unwrap(),
        );
        let directory = std::path::PathBuf::from(std::env::var("ADVANCE_HUD_SCENES").unwrap());
        let mut checked = 0;
        for stem in [
            "001675", "001850", "002100", "002600", "002910", "002950", "003450",
        ] {
            let mut source = MemMap::new(Vec::new());
            for (ext, target) in [
                ("ewram", &mut source.ewram),
                ("iwram", &mut source.iwram),
                ("vram", &mut source.vram),
                ("io", &mut source.io),
                ("palette", &mut source.palette),
            ] {
                target.copy_from_slice(
                    &std::fs::read(directory.join(format!("{stem}.{ext}"))).unwrap(),
                );
            }
            let world = WorldGround::capture(&source, rom.clone()).unwrap();
            let [rx, ry, pa, pd] = continuous_parameters(&source);
            for y in 0..160 {
                for x in 0..240 {
                    let tx = ((rx + pa * x as f64) / 256.0).floor() as i32;
                    let ty = ((ry + pd * y as f64) / 256.0).floor() as i32;
                    let resident = texel_layer(&source, tx, ty);
                    if resident.is_some() {
                        assert_eq!(world.pixel(&source, tx, ty), resident, "{stem} {tx},{ty}");
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 250000);
        println!("world_ground verified {checked} resident samples");
    }
    #[test]
    fn wide_ground_center_is_identical_and_sides_use_new_world_coordinates() {
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
        let view = crate::viewport::View::new(3, crate::viewport::Aspect::Wide);
        for zoom in [0.95, 1.0, 1.05] {
            let original = continuous_layer_zoom(&source, 3, [32768, -16384], zoom);
            let wide = continuous_layer_view(&source, view, [32768, -16384], zoom);
            for y in 0..480 {
                for x in 0..720 {
                    assert_eq!(
                        original[y * 720 + x],
                        wide[y * view.width + x + view.padding]
                    );
                }
            }
        }
    }
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
