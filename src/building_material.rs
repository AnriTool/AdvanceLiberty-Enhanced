//! USA scene-copy material capture and offline affine triangle rendering.
//! No guest writes. Pixel coverage is a new rasterizer, not a bit-exact GBA model.
use crate::building_projection::{bytes, word};
use gba_core::{ppu, Machine, MemMap};
use std::{collections::HashMap, io::Write, path::Path, sync::Arc};

#[derive(Clone)]
pub struct Input {
    source: u32,
    color: u8,
    size: usize,
    uv: Vec<[f64; 2]>,
}
impl Input {
    pub fn uv64(&self, i: usize) -> Option<[i32; 2]> {
        if self.size == 64 {
            self.uv.get(i).map(|uv| uv.map(|v| v as i32))
        } else {
            None
        }
    }
    pub fn read(m: &Machine, address: u32, count: usize) -> Result<Self, String> {
        let header = bytes(
            m,
            address.checked_sub(4).ok_or("Material header underflow")?,
            4,
        )?;
        let size = if count == 4 && header[3] & 8 == 0 {
            128
        } else {
            64
        };
        let uv = if header[3] & 16 != 0 {
            bytes(m, address + count as u32 * 6, count * 2)?
                .chunks_exact(2)
                .map(|p| {
                    [
                        p[0] as f64,
                        (size - 1 + usize::from(size == 128)) as f64 - p[1] as f64,
                    ]
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            source: address,
            color: header[2],
            size,
            uv,
        })
    }
}
#[derive(Clone)]
struct Face {
    vertices: Vec<[f64; 5]>,
    input: Input,
    texture: Arc<Vec<u8>>,
}
#[derive(Clone, Copy, Default, Debug)]
struct Orientation {
    rotation: u8,
    mirror_u: bool,
    mirror_v: bool,
}
impl Orientation {
    fn apply(self, [u, v]: [f64; 2]) -> [f64; 2] {
        let [mut u, mut v] = match self.rotation {
            1 => [128.0 - v, u],
            2 => [v, 128.0 - u],
            _ => [u, v],
        };
        if self.mirror_u {
            u = 128.0 - u;
        }
        if self.mirror_v {
            v = 128.0 - v;
        }
        [u, v]
    }
}
#[derive(Clone, Default)]
pub struct Capture {
    faces: Vec<Face>,
    group_start: usize,
    descriptor: u32,
    orientation: bool,
    special: bool,
    quads: HashMap<u32, usize>,
    rows: Vec<String>,
    palette: Vec<u8>,
}
impl Capture {
    pub fn supports_camera(&self, delta: [i32; 3], scale: usize) -> bool {
        self.faces
            .iter()
            .filter(|f| !f.input.uv.is_empty() && f.texture.is_empty())
            .all(|face| {
                let mut vertices = face.vertices.clone();
                for v in &mut vertices {
                    v[0] -= delta[0] as f64 / 65536.0;
                    v[1] += delta[1] as f64 / 65536.0;
                    v[2] += delta[2] as f64 / 65536.0;
                }
                !polygon_has_samples(&clip(vertices), scale)
            })
    }
    pub fn restore_edge_materials(&mut self, previous: &Self, camera_delta: [i32; 3]) -> usize {
        if !self
            .faces
            .iter()
            .any(|f| f.texture.is_empty() && !f.input.uv.is_empty())
        {
            return 0;
        }
        let mut assets: HashMap<u32, Vec<&Face>> = HashMap::new();
        for face in &previous.faces {
            if !face.texture.is_empty() {
                assets.entry(face.input.source).or_default().push(face);
            }
        }
        let shift = camera_delta.map(|v| v as f64 / 65536.0);
        let mut restored = 0;
        for face in &mut self.faces {
            if !face.texture.is_empty() || face.input.uv.is_empty() || face.input.source >> 24 != 8
            {
                continue;
            }
            if let Some(old) = assets
                .get(&face.input.source)
                .into_iter()
                .flatten()
                .copied()
                .find(|old| {
                    !old.texture.is_empty()
                        && old.input.source == face.input.source
                        && old.input.size == face.input.size
                        && old.input.color == face.input.color
                        && old.input.uv == face.input.uv
                        && old.vertices.len() == face.vertices.len()
                        && old.vertices.iter().zip(&face.vertices).all(|(a, b)| {
                            let translated = [a[0] - shift[0], a[1] + shift[1], a[2] + shift[2]];
                            translated
                                .iter()
                                .zip(&b[..3])
                                .all(|(x, y)| (x - y).abs() < 1e-7)
                        })
                })
            {
                face.texture = old.texture.clone();
                // A 128x128 quad's submitted texture and transformed UVs are
                // an inseparable pair (rotation/alternate atlas). Keep both.
                for (vertex, old_vertex) in face.vertices.iter_mut().zip(&old.vertices) {
                    vertex[3..5].copy_from_slice(&old_vertex[3..5]);
                }
                restored += 1;
            }
        }
        restored
    }
    #[inline]
    pub fn interested(key: u32) -> bool {
        matches!(key, 0x0800db65 | 0x0800e249 | 0x0800dfc9)
    }
    pub fn translated(&self, camera_delta: [i32; 3]) -> Self {
        let mut result = self.clone();
        for face in &mut result.faces {
            for v in &mut face.vertices {
                v[0] -= camera_delta[0] as f64 / 65536.0;
                v[1] += camera_delta[1] as f64 / 65536.0;
                v[2] += camera_delta[2] as f64 / 65536.0;
            }
        }
        result
    }
    pub fn live_layer(&self, delta: [i32; 2], scale: usize) -> Result<Vec<u8>, String> {
        self.live_layer_mapped(delta, scale, crate::render_settings::TextureMapping::Affine)
    }
    pub fn live_layer_mapped(
        &self,
        delta: [i32; 2],
        scale: usize,
        mapping: crate::render_settings::TextureMapping,
    ) -> Result<Vec<u8>, String> {
        self.live_layer_camera([delta[0], delta[1], 0], scale, mapping)
    }
    pub fn live_layer_camera(
        &self,
        delta: [i32; 3],
        scale: usize,
        mapping: crate::render_settings::TextureMapping,
    ) -> Result<Vec<u8>, String> {
        // A model fully culled by the guest can omit its draw-batch call.
        // Only a visible textured face needs a captured texture here. Flat
        // faces use the scene palette supplied by the live compositor.
        let mut layer = vec![0; 240 * 160 * scale * scale];
        for face in &self.faces {
            let mut vertices = face.vertices.clone();
            for v in &mut vertices {
                v[0] -= delta[0] as f64 / 65536.0;
                v[1] += delta[1] as f64 / 65536.0;
                v[2] += delta[2] as f64 / 65536.0;
            }
            let vertices = clip(vertices);
            if vertices.len() < 3 {
                continue;
            }
            if !face.input.uv.is_empty() && face.texture.is_empty() {
                if polygon_has_samples(&vertices, scale) {
                    return Err("Visible face has no captured material".into());
                }
                continue;
            }
            let v: Vec<_> = vertices
                .iter()
                .map(|v| {
                    [
                        120.0 + 118.0 * v[0] / v[2],
                        120.0 - 118.0 * v[1] / v[2],
                        v[3],
                        v[4],
                    ]
                })
                .collect();
            for i in 1..v.len() - 1 {
                let triangle_vertices = [v[0], v[i], v[i + 1]];
                if mapping == crate::render_settings::TextureMapping::Perspective {
                    triangle_perspective(
                        &mut layer,
                        scale,
                        triangle_vertices,
                        [vertices[0][2], vertices[i][2], vertices[i + 1][2]],
                        face,
                    );
                } else {
                    triangle(&mut layer, scale, triangle_vertices, face);
                }
            }
        }
        Ok(layer)
    }
    pub fn push(&mut self, vertices: Vec<[f64; 3]>, input: Input) {
        let vertices = vertices
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let uv = input.uv.get(i).copied().unwrap_or([0.0; 2]);
                [p[0], p[1], p[2], uv[0], uv[1]]
            })
            .collect();
        self.faces.push(Face {
            vertices,
            input,
            texture: Arc::new(Vec::new()),
        });
    }
    pub fn observe(&mut self, m: &Machine, key: u32) -> Result<(), String> {
        if key == 0x0800db65 {
            self.descriptor = m.cpu.regs[0];
            self.orientation = m.cpu.regs[2] & 1 != 0;
            self.special = m.cpu.regs[3] & 1 != 0;
            let texture = Arc::new(if m.cpu.regs[1] == 0 {
                Vec::new()
            } else {
                bytes(m, m.cpu.regs[1], 4096)?.to_vec()
            });
            for face in &mut self.faces[self.group_start..] {
                if !face.input.uv.is_empty() && face.input.size == 64 {
                    if texture.is_empty() {
                        return Err("Missing 64x64 model texture".into());
                    }
                    face.texture = texture.clone();
                }
            }
            self.group_start = self.faces.len();
            self.palette = m.bus.palette.clone();
        }
        if key == 0x0800e249 {
            let index = self
                .faces
                .len()
                .checked_sub(1)
                .ok_or("Descriptor without source face")?;
            self.quads.insert(word(m, 0x03000020)? as u32, index);
        }
        if key == 0x0800dfc9 && m.cpu.regs[6] != 0 {
            let descriptor = m.cpu.regs[4];
            let index = *self
                .quads
                .get(&descriptor)
                .ok_or("Raster quad without source face")?;
            let rotated = bytes(m, descriptor + 1, 1)?[0] != 0;
            let point = |offset| -> Result<[i32; 4], String> {
                Ok([
                    word(m, descriptor + offset)?,
                    word(m, descriptor + offset + 4)?,
                    word(m, descriptor + offset + 8)?,
                    128 - word(m, descriptor + offset + 12)?,
                ])
            };
            let (mut a, mut b) = (point(8)?, point(40)?);
            if a[1] >= b[1] {
                std::mem::swap(&mut a, &mut b);
            }
            let mut transform = Orientation {
                rotation: if rotated {
                    if self.orientation {
                        2
                    } else {
                        1
                    }
                } else {
                    0
                },
                ..Orientation::default()
            };
            let uv = |p: [i32; 4]| [p[2].clamp(0, 128) as f64, p[3].clamp(0, 128) as f64];
            let (start, end) = (transform.apply(uv(a)), transform.apply(uv(b)));
            let mut alternate = if rotated {
                !self.orientation
            } else {
                self.orientation
            };
            if self.special {
                transform.mirror_u =
                    (a[0] < b[0] && start[0] > end[0]) || (a[0] > b[0] && start[0] < end[0]);
                transform.mirror_v = start[1] > end[1];
                alternate = false;
            }
            let mip = m.cpu.regs[7];
            if mip > 2 {
                return Err(format!("Unsupported material mip {mip}"));
            }
            let selected =
                word(m, self.descriptor + mip * 8 + if alternate { 4 } else { 0 })? as u32;
            if selected != m.cpu.regs[6] {
                return Err("Material texture pointer mismatch".into());
            }
            let mut checked_uv = Vec::new();
            for (p, offset) in [(a, 0), (b, 8)] {
                let expected = transform.apply(uv(p)).map(|v| v as i32 >> mip);
                let actual = [
                    word(m, m.cpu.regs[5] + offset)?,
                    word(m, m.cpu.regs[5] + offset + 4)?,
                ];
                if expected != actual {
                    return Err(format!("Material UV mismatch: {expected:?} != {actual:?}"));
                }
                checked_uv.extend(actual);
            }
            let full = word(m, self.descriptor + if alternate { 4 } else { 0 })? as u32;
            let face = &mut self.faces[index];
            if face.input.size != 128 || face.input.uv.is_empty() || !face.texture.is_empty() {
                return Err("Inconsistent quad material/source association".into());
            }
            face.texture = Arc::new(bytes(m, full, 128 * 128)?.to_vec());
            for vertex in &mut face.vertices {
                let p = transform.apply([vertex[3], vertex[4]]);
                vertex[3] = p[0];
                vertex[4] = p[1];
            }
            self.rows.push(format!("{index},{descriptor:08x},{selected:08x},{full:08x},{mip},{},{},{},{alternate},{},{},{},{}",transform.rotation,transform.mirror_u,transform.mirror_v,checked_uv[0],checked_uv[1],checked_uv[2],checked_uv[3]));
        }
        Ok(())
    }
    fn render(
        &self,
        source: &MemMap,
        delta: i32,
        scale: usize,
    ) -> (Vec<u16>, usize, usize, Vec<u8>) {
        let mut image = crate::ground_projection::continuous_image(source, scale, delta, 0);
        let mut layer = vec![0u8; image.len()];
        let mut textured = 0;
        let mut flat = 0;
        for face in &self.faces {
            if !face.input.uv.is_empty() && face.texture.is_empty() {
                continue;
            }
            let mut vertices = face.vertices.clone();
            for v in &mut vertices {
                v[0] -= delta as f64 / 65536.0;
            }
            let vertices = clip(vertices);
            if vertices.len() < 3 {
                continue;
            }
            if face.texture.is_empty() {
                flat += 1;
            } else {
                textured += 1;
            }
            let projected: Vec<_> = vertices
                .iter()
                .map(|v| {
                    [
                        120.0 + 118.0 * v[0] / v[2],
                        120.0 - 118.0 * v[1] / v[2],
                        v[3],
                        v[4],
                    ]
                })
                .collect();
            for i in 1..projected.len() - 1 {
                triangle(
                    &mut layer,
                    scale,
                    [projected[0], projected[i], projected[i + 1]],
                    face,
                );
            }
        }
        for (p, index) in image.iter_mut().zip(layer.iter().copied()) {
            if index != 0 {
                let offset = index as usize * 2;
                *p = u16::from_le_bytes(self.palette[offset..offset + 2].try_into().unwrap());
            }
        }
        (image, textured, flat, layer)
    }
    pub fn export(
        &self,
        source: &MemMap,
        dir: &Path,
        objects: &crate::object_projection::Snapshot,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.group_start != self.faces.len() {
            return Err("Material model batch not finalized".into());
        }
        if self.palette.len() < 512 {
            return Err("No observed material palette".into());
        }
        std::fs::create_dir(dir)?;
        // Raw BG0+BG2 control image. OAM, HUD, blending and windows are omitted
        // deliberately in this comparison, just as in the fractional output.
        let mut gpu = MemMap::new(Vec::new());
        gpu.io.clone_from(&source.io);
        gpu.vram.clone_from(&source.vram);
        gpu.palette.clone_from(&source.palette);
        gpu.io[..2].copy_from_slice(&0x0501u16.to_le_bytes());
        gpu.io[0x50..0x56].fill(0);
        gpu.aff_dirty = [true; 2];
        for y in 0..160 {
            ppu::render_scanline(&mut gpu, y);
        }
        let reference: Vec<u16> = (0..480)
            .flat_map(|y| {
                let framebuffer = &gpu.framebuffer;
                (0..720).map(move |x| framebuffer[(y / 3) * 240 + x / 3])
            })
            .collect();
        crate::scene_replay::ppm(&dir.join("reference-bg0-bg2.ppm"), &reference, 720, 480)?;
        let sx = u16::from_le_bytes(source.io[0x10..0x12].try_into().unwrap()) & 511;
        let sy = u16::from_le_bytes(source.io[0x12..0x14].try_into().unwrap()) & 511;
        if sx != 0 || sy != 0 {
            return Err("Fractional material composition requires zero BG0 scroll".into());
        }
        let mut checks = std::fs::File::create_new(dir.join("quad-checks.csv"))?;
        writeln!(checks,"face,descriptor,selected_texture,full_texture,mip,rotation,mirror_u,mirror_v,alternate,u0,v0,u1,v1")?;
        for row in &self.rows {
            writeln!(checks, "{row}")?;
        }
        let mut faces = std::fs::File::create_new(dir.join("faces.csv"))?;
        writeln!(
            faces,
            "face,vertices,texture_size,textured,texture_resolved,flat_index"
        )?;
        for (i, f) in self.faces.iter().enumerate() {
            writeln!(
                faces,
                "{i},{},{},{},{},{}",
                f.vertices.len(),
                f.input.size,
                !f.input.uv.is_empty(),
                !f.texture.is_empty(),
                f.input.color
            )?;
            if !f.texture.is_empty() {
                std::fs::write(
                    dir.join(format!("texture-{i:03}.indexed8")),
                    f.texture.as_slice(),
                )?;
            }
        }
        std::fs::write(dir.join("palette.bin"), &self.palette)?;
        let unresolved64 = self
            .faces
            .iter()
            .filter(|f| f.input.size == 64 && !f.input.uv.is_empty() && f.texture.is_empty())
            .count();
        if unresolved64 != 0 {
            return Err("Unresolved 64x64 material".into());
        }
        let mut motions = std::fs::File::create_new(dir.join("camera.csv"))?;
        writeln!(
            motions,
            "delta_x_raw,textured_faces,flat_faces,changed_pixels_3x"
        )?;
        let mut base = None;
        for delta in [0, 16384, 32768, 49152, 65536] {
            let (image, textured, flat, layer) = self.render(source, delta, 3);
            let scene = objects.compose(source, &layer, &self.palette, delta, 3);
            crate::scene_replay::ppm(&dir.join(format!("scene-{delta}.ppm")), &scene, 720, 480)?;
            std::fs::write(dir.join(format!("building-{delta}.indexed8")), &layer)?;
            let changed = base
                .as_ref()
                .map(|b: &Vec<u16>| b.iter().zip(&image).filter(|(a, b)| a != b).count())
                .unwrap_or(0);
            crate::scene_replay::ppm(&dir.join(format!("textured-{delta}.ppm")), &image, 720, 480)?;
            writeln!(motions, "{delta},{textured},{flat},{changed}")?;
            if delta == 0 {
                base = Some(image);
            }
        }
        let skipped = self
            .faces
            .iter()
            .filter(|f| !f.input.uv.is_empty() && f.texture.is_empty())
            .count();
        std::fs::write(dir.join("limits.txt"),format!("quad_texture_and_uv_checks={}\nsource_faces={}\nskipped_unresolved_128_faces={skipped}\ninterpolation=affine, full-resolution textures, nearest texel\norder=source face order; original face selection\nindex0=transparent; no OBJ/HUD; pixel coverage not original\n",self.rows.len(),self.faces.len()))?;
        Ok(())
    }
}

// The homogeneous clip volume is taller than the 160-line viewport. A
// surviving polygon may still cover no output samples, including thin edges.
fn polygon_has_samples(vertices: &[[f64; 5]], scale: usize) -> bool {
    if vertices.len() < 3 {
        return false;
    }
    let project = |v: &[f64; 5]| [120.0 + 118.0 * v[0] / v[2], 120.0 - 118.0 * v[1] / v[2]];
    (1..vertices.len() - 1).any(|i| {
        triangle_has_samples(
            [
                project(&vertices[0]),
                project(&vertices[i]),
                project(&vertices[i + 1]),
            ],
            scale,
        )
    })
}
fn triangle_has_samples(v: [[f64; 2]; 3], scale: usize) -> bool {
    let edge = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };
    let area = edge(v[0], v[1], v[2]);
    if area.abs() < 1e-12 {
        return false;
    }
    let lo = |axis: usize| {
        (v.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min) * scale as f64)
            .floor()
            .max(0.0) as usize
    };
    let hi = |axis: usize, limit: usize| {
        (v.iter().map(|p| p[axis]).fold(f64::NEG_INFINITY, f64::max) * scale as f64)
            .ceil()
            .clamp(0.0, limit as f64) as usize
    };
    for y in lo(1)..hi(1, 160 * scale) {
        for x in lo(0)..hi(0, 240 * scale) {
            let p = [
                (x as f64 + 0.5) / scale as f64,
                (y as f64 + 0.5) / scale as f64,
            ];
            if [(v[1], v[2]), (v[2], v[0]), (v[0], v[1])]
                .iter()
                .all(|&(a, b)| edge(a, b, p) / area >= -1e-10)
            {
                return true;
            }
        }
    }
    false
}
fn clip(mut vertices: Vec<[f64; 5]>) -> Vec<[f64; 5]> {
    for plane in 0..5 {
        let distance = |v: [f64; 5]| match plane {
            0 => v[2] - 32.0,
            1 => v[2] + v[0],
            2 => v[2] - v[0],
            3 => v[2] + v[1],
            _ => v[2] - v[1],
        };
        let Some(mut a) = vertices.last().copied() else {
            break;
        };
        let mut output = Vec::new();
        for b in vertices {
            let (da, db) = (distance(a), distance(b));
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                output.push(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t));
            }
            if db >= 0.0 {
                output.push(b);
            }
            a = b;
        }
        vertices = output;
    }
    vertices
}
fn triangle(image: &mut [u8], scale: usize, v: [[f64; 4]; 3], face: &Face) {
    raster_triangle(image, scale, v, face, true);
}
fn raster_triangle(image: &mut [u8], scale: usize, v: [[f64; 4]; 3], face: &Face, spans: bool) {
    raster_triangle_impl::<false>(image, scale, v, [1.0; 3], face, spans);
}
fn triangle_perspective(
    image: &mut [u8],
    scale: usize,
    v: [[f64; 4]; 3],
    depth: [f64; 3],
    face: &Face,
) {
    if depth[0] == depth[1] && depth[1] == depth[2] {
        triangle(image, scale, v, face);
    } else {
        raster_triangle_impl::<true>(image, scale, v, depth, face, true);
    }
}
fn raster_triangle_impl<const PERSPECTIVE: bool>(
    image: &mut [u8],
    scale: usize,
    v: [[f64; 4]; 3],
    depth: [f64; 3],
    face: &Face,
    spans: bool,
) {
    let inverse_depth = depth.map(|z| 1.0 / z);
    let width = 240 * scale;
    let height = 160 * scale;
    let edge = |a: [f64; 4], b: [f64; 4], p: [f64; 2]| {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };
    let area = edge(v[0], v[1], [v[2][0], v[2][1]]);
    if area.abs() < 1e-12 {
        return;
    }
    let lo = |axis: usize| {
        (v.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min) * scale as f64)
            .floor()
            .max(0.0) as usize
    };
    let hi = |axis: usize, limit: usize| {
        (v.iter().map(|p| p[axis]).fold(f64::NEG_INFINITY, f64::max) * scale as f64)
            .ceil()
            .clamp(0.0, limit as f64) as usize
    };
    for y in lo(1)..hi(1, height) {
        let (mut start, mut end) = (lo(0), hi(0, width));
        if spans {
            // Intersect this scanline with all three half planes. Keep a
            // one-pixel margin and the original barycentric test below so
            // coverage and UV rounding remain identical at triangle edges.
            let py = (y as f64 + 0.5) / scale as f64;
            let (mut left, mut right) = (0.0f64, 240.0f64);
            for (a, b) in [(v[1], v[2]), (v[2], v[0]), (v[0], v[1])] {
                let slope = -(b[1] - a[1]) / area;
                let origin = edge(a, b, [0.0, py]) / area;
                if slope > 0.0 {
                    left = left.max((-1e-10 - origin) / slope);
                } else if slope < 0.0 {
                    right = right.min((-1e-10 - origin) / slope);
                } else if origin < -1e-10 {
                    right = -1.0;
                }
            }
            if right < left {
                continue;
            }
            start = start.max(((left * scale as f64 - 0.5).floor() - 1.0).max(0.0) as usize);
            end = end.min(((right * scale as f64 - 0.5).ceil() + 2.0).max(0.0) as usize);
        }
        for x in start..end {
            let p = [
                (x as f64 + 0.5) / scale as f64,
                (y as f64 + 0.5) / scale as f64,
            ];
            let w = [
                edge(v[1], v[2], p) / area,
                edge(v[2], v[0], p) / area,
                edge(v[0], v[1], p) / area,
            ];
            if w.iter().any(|n| *n < -1e-10) {
                continue;
            }
            let index = if face.texture.is_empty() {
                face.input.color
            } else {
                let weights = if PERSPECTIVE {
                    let weighted: [f64; 3] = std::array::from_fn(|i| w[i] * inverse_depth[i]);
                    let sum: f64 = weighted.iter().sum();
                    weighted.map(|weight| weight / sum)
                } else {
                    w
                };
                let uv: [usize; 2] = std::array::from_fn(|axis| {
                    (0..3)
                        .map(|i| weights[i] * v[i][axis + 2])
                        .sum::<f64>()
                        .floor()
                        .clamp(0.0, (face.input.size - 1) as f64) as usize
                });
                face.texture[uv[1] * face.input.size + uv[0]]
            };
            image[y * width + x] = index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn material_requirement_matches_actual_pixel_coverage() {
        let face = Face {
            vertices: Vec::new(),
            input: Input {
                source: 0,
                color: 1,
                size: 64,
                uv: Vec::new(),
            },
            texture: Arc::new(Vec::new()),
        };
        let mut seed = 19u32;
        for scale in [1, 3] {
            for case in 0..160 {
                let v: [[f64; 4]; 3] = std::array::from_fn(|_| {
                    let xy: [f64; 2] = std::array::from_fn(|_| {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        (seed % 900) as f64 / 3.0 - 30.0
                    });
                    [xy[0], xy[1], 0.0, 0.0]
                });
                let mut image = vec![0; 38400 * scale * scale];
                triangle(&mut image, scale, v, &face);
                assert_eq!(
                    triangle_has_samples(v.map(|p| [p[0], p[1]]), scale),
                    image.iter().any(|&p| p != 0),
                    "scale={scale}, case={case}"
                );
            }
            assert!(!triangle_has_samples(
                [[0.0, 180.0], [100.0, 170.0], [0.0, 170.0]],
                scale
            ));
            assert!(!triangle_has_samples([[1.0, 1.0]; 3], scale));
            assert!(!triangle_has_samples(
                [[0.0, 0.0], [0.01, 0.0], [0.0, 0.01]],
                scale
            ));
        }
        let mut capture = Capture::default();
        capture.push(
            vec![
                [0.0, -80.0, 100.0],
                [10.0, -80.0, 100.0],
                [0.0, -90.0, 100.0],
            ],
            Input {
                source: 0,
                color: 1,
                size: 64,
                uv: vec![[0.0; 2]; 3],
            },
        );
        assert!(capture.supports_camera([0; 3], 3));
        assert!(capture
            .live_layer([0; 2], 3)
            .unwrap()
            .iter()
            .all(|&p| p == 0));
        capture.faces[0].vertices = vec![
            [0.0, 0.0, 100.0, 0.0, 0.0],
            [10.0, 0.0, 100.0, 0.0, 0.0],
            [0.0, 10.0, 100.0, 0.0, 0.0],
        ];
        assert!(!capture.supports_camera([0; 3], 3));
        assert!(capture.live_layer([0; 2], 3).is_err());
    }
    #[test]
    fn restored_quad_keeps_transformed_uv_and_selected_texture_together() {
        let mut old = Capture::default();
        old.push(
            vec![[80.0, 0.0, 100.0], [110.0, 0.0, 100.0], [80.0, 30.0, 100.0]],
            Input {
                source: 0x08000104,
                color: 1,
                size: 128,
                uv: vec![[0.0, 0.0], [128.0, 0.0], [0.0, 128.0]],
            },
        );
        old.faces[0].texture = Arc::new((0..16384).map(|i| (i % 251 + 1) as u8).collect());
        for v in &mut old.faces[0].vertices {
            let (u, w) = (v[3], v[4]);
            v[3] = 128.0 - w;
            v[4] = u;
        }
        let shift = [-80 * 65536, 0, 0];
        let mut current = old.translated(shift);
        current.faces[0].texture = Arc::new(Vec::new());
        for i in 0..3 {
            let uv = current.faces[0].input.uv[i];
            current.faces[0].vertices[i][3..5].copy_from_slice(&uv);
        }
        assert_eq!(current.restore_edge_materials(&old, shift), 1);
        assert_eq!(
            current.live_layer([80 * 65536, 0], 1).unwrap(),
            old.live_layer([0; 2], 1).unwrap()
        );
    }
    #[test]
    fn culled_edge_reuses_only_the_same_asset_face_at_the_same_world_position() {
        let mut old = Capture::default();
        old.push(
            vec![[80.0, 0.0, 100.0], [110.0, 0.0, 100.0], [80.0, 30.0, 100.0]],
            Input {
                source: 0x08000104,
                color: 1,
                size: 64,
                uv: vec![[0.0, 0.0], [63.0, 0.0], [0.0, 63.0]],
            },
        );
        old.faces[0].texture = Arc::new(vec![3; 4096]);
        let shift = [-80 * 65536, 0, 0];
        let mut current = old.translated(shift);
        current.faces[0].texture = Arc::new(Vec::new());
        let missing = current.clone();
        assert!(current.live_layer([0; 2], 1).is_ok());
        assert!(current.live_layer([80 * 65536, 0], 1).is_err());
        assert_eq!(current.restore_edge_materials(&old, shift), 1);
        assert!(Arc::ptr_eq(
            &current.faces[0].texture,
            &old.faces[0].texture
        ));
        assert_eq!(
            current.live_layer([80 * 65536, 0], 1).unwrap(),
            old.live_layer([0; 2], 1).unwrap()
        );
        for variant in 0..5 {
            let mut different = missing.clone();
            match variant {
                0 => different.faces[0].input.source += 4,
                1 => different.faces[0].vertices[0][0] += 0.01,
                2 => different.faces[0].input.uv[0][0] += 1.0,
                3 => different.faces[0].input.color += 1,
                _ => different.faces[0].input.source = 0x02000104,
            }
            assert_eq!(different.restore_edge_materials(&old, shift), 0);
            assert!(different.faces[0].texture.is_empty());
        }
    }
    #[test]
    fn camera_height_reprojects_faces_by_depth_instead_of_scaling_the_final_image() {
        let mut capture = Capture::default();
        capture.push(
            vec![[0.0, 0.0, 100.0], [40.0, 0.0, 200.0], [0.0, 40.0, 100.0]],
            Input {
                source: 0,
                color: 1,
                size: 64,
                uv: Vec::new(),
            },
        );
        let mapping = crate::render_settings::TextureMapping::Perspective;
        let original = capture.live_layer_mapped([0; 2], 3, mapping).unwrap();
        let farther = capture
            .live_layer_camera([0, 0, 100 * 65536], 3, mapping)
            .unwrap();
        let area = |v: &[u8]| v.iter().filter(|&&p| p != 0).count();
        assert!(area(&farther) > area(&original) / 4);
        assert!(area(&farther) < area(&original) / 2);
        assert_eq!(
            original,
            capture.live_layer_camera([0; 3], 3, mapping).unwrap()
        );
        assert_eq!(
            farther,
            capture
                .translated([0, 0, 100 * 65536])
                .live_layer_mapped([0; 2], 3, mapping)
                .unwrap()
        );
    }
    #[test]
    fn perspective_uv_uses_depth_and_equal_depth_keeps_affine_pixels() {
        let face = Face {
            vertices: Vec::new(),
            input: Input {
                source: 0,
                color: 1,
                size: 8,
                uv: vec![[0.0; 2]; 3],
            },
            texture: Arc::new((1..=64).collect()),
        };
        let v = [
            [0.0, 0.0, 0.0, 0.0],
            [8.0, 0.0, 8.0, 0.0],
            [0.0, 8.0, 0.0, 8.0],
        ];
        let mut affine = vec![0; 38400];
        let mut perspective = affine.clone();
        triangle(&mut affine, 1, v, &face);
        triangle_perspective(&mut perspective, 1, v, [32.0, 64.0, 64.0], &face);
        assert_eq!(affine[2 * 240 + 2], 19);
        assert_eq!(perspective[2 * 240 + 2], 10);
        triangle_perspective(&mut perspective, 1, v, [64.0; 3], &face);
        assert_eq!(perspective, affine);
    }
    #[test]
    fn perspective_planar_quad_does_not_depend_on_triangulation_diagonal() {
        let face = Face {
            vertices: Vec::new(),
            input: Input {
                source: 0,
                color: 1,
                size: 8,
                uv: vec![[0.0; 2]; 4],
            },
            texture: Arc::new((1..=64).collect()),
        };
        let world = [
            [-8.0, -8.0, 32.0],
            [8.0, -8.0, 64.0],
            [8.0, 8.0, 64.0],
            [-8.0, 8.0, 32.0],
        ];
        let uv = [
            [0.137, 0.173],
            [7.137, 0.173],
            [7.137, 7.173],
            [0.137, 7.173],
        ];
        let v: [[f64; 4]; 4] = std::array::from_fn(|i| {
            [
                120.0 + 118.0 * world[i][0] / world[i][2],
                120.0 - 118.0 * world[i][1] / world[i][2],
                uv[i][0],
                uv[i][1],
            ]
        });
        let mut a = vec![0; 38400 * 9];
        let mut b = a.clone();
        for (out, triangles) in [
            (&mut a, [[0, 1, 2], [0, 2, 3]]),
            (&mut b, [[0, 1, 3], [1, 2, 3]]),
        ] {
            for t in triangles {
                triangle_perspective(out, 3, t.map(|i| v[i]), t.map(|i| world[i][2]), &face);
            }
        }
        assert_eq!(a, b);
    }
    #[test]
    fn cached_geometry_tracks_camera_on_all_axes_and_shares_only_texture_storage() {
        let mut original = Capture::default();
        original.push(
            vec![[1.0, 2.0, 100.0]; 3],
            Input {
                source: 0,
                color: 1,
                size: 64,
                uv: Vec::new(),
            },
        );
        original.faces[0].texture = Arc::new(vec![3; 4096]);
        let translated = original.translated([32768, -65536, 16384]);
        assert_eq!(&translated.faces[0].vertices[0][..3], &[0.5, 1.0, 100.25]);
        assert_eq!(&original.faces[0].vertices[0][..3], &[1.0, 2.0, 100.0]);
        assert!(Arc::ptr_eq(
            &original.faces[0].texture,
            &translated.faces[0].texture
        ));
        assert_eq!(
            translated.translated([-32768, 65536, -16384]).faces[0].vertices,
            original.faces[0].vertices
        );
    }
    #[test]
    fn scanline_spans_match_full_barycentric_coverage_and_texels() {
        let face = Face {
            vertices: Vec::new(),
            input: Input {
                source: 0,
                color: 7,
                size: 64,
                uv: vec![[0.0; 2]; 3],
            },
            texture: Arc::new((0..4096).map(|i| (i % 251) as u8).collect()),
        };
        let mut seed = 19731u32;
        for scale in [1, 3] {
            for _ in 0..120 {
                let v = std::array::from_fn(|_| {
                    std::array::from_fn(|axis| {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        let t = seed as f64 / u32::MAX as f64;
                        match axis {
                            0 => t * 300.0 - 30.0,
                            1 => t * 220.0 - 30.0,
                            _ => t * 90.0 - 10.0,
                        }
                    })
                });
                let mut expected = vec![255; 38400 * scale * scale];
                let mut actual = expected.clone();
                raster_triangle(&mut expected, scale, v, &face, false);
                triangle(&mut actual, scale, v, &face);
                assert_eq!(actual, expected, "scale={scale}, vertices={v:?}");
            }
        }
    }
    #[test]
    fn unsubmitted_culled_batch_does_not_hide_the_scene_but_visible_texture_is_required() {
        let mut capture = Capture::default();
        capture.push(
            vec![
                [1000.0, 0.0, 100.0],
                [1010.0, 0.0, 100.0],
                [1000.0, 10.0, 100.0],
            ],
            Input {
                source: 0,
                color: 1,
                size: 64,
                uv: vec![[0.0; 2]; 3],
            },
        );
        assert!(capture.live_layer([0; 2], 1).is_ok());
        capture.faces[0].vertices = vec![
            [-5.0, 0.0, 100.0, 0.0, 0.0],
            [5.0, 0.0, 100.0, 0.0, 0.0],
            [0.0, 5.0, 100.0, 0.0, 0.0],
        ];
        assert!(capture.live_layer([0; 2], 1).is_err());
        capture.faces[0].input.uv.clear();
        assert!(capture.live_layer([0; 2], 1).unwrap().contains(&1));
    }
    #[test]
    fn clipping_interpolates_uv_at_near_plane() {
        let v = clip(vec![
            [-4.0, 0.0, 16.0, 0.0, 0.0],
            [4.0, 0.0, 48.0, 64.0, 0.0],
            [0.0, 4.0, 48.0, 32.0, 64.0],
        ]);
        assert!(v
            .iter()
            .any(|p| p[2] == 32.0 && p[3] == 32.0 && p[4] == 0.0));
        for p in v {
            assert!(p[2] >= 32.0);
        }
    }
    #[test]
    fn orientation_keeps_fractional_coordinates() {
        let p = [12.25, 63.75];
        assert_eq!(
            Orientation {
                rotation: 1,
                ..Orientation::default()
            }
            .apply(p),
            [64.25, 12.25]
        );
        assert_eq!(
            Orientation {
                rotation: 2,
                mirror_u: true,
                mirror_v: true
            }
            .apply(p),
            [64.25, 12.25]
        );
    }
    #[test]
    fn triangle_pair_fills_quad_and_zero_texel_erases_prior_geometry() {
        let mut image = vec![2u8; 240 * 160];
        let face = Face {
            vertices: Vec::new(),
            input: Input {
                source: 0,
                color: 0,
                size: 2,
                uv: vec![[0.0; 2]; 4],
            },
            texture: Arc::new(vec![1, 0, 1, 1]),
        };
        let [a, b, c, d] = [
            [0., 0., 0., 0.],
            [4., 0., 2., 0.],
            [4., 4., 2., 2.],
            [0., 4., 0., 2.],
        ];
        triangle(&mut image, 1, [a, b, c], &face);
        triangle(&mut image, 1, [a, c, d], &face);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(image[y * 240 + x], if y < 2 && x >= 2 { 0 } else { 1 });
            }
        }
        assert_eq!(image[4], 2);
    }
}
