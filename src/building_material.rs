//! USA scene-copy material capture and offline affine triangle rendering.
//! No guest writes. Pixel coverage is a new rasterizer, not a bit-exact GBA model.
use crate::building_projection::{bytes, word};
use gba_core::{ppu, Machine, MemMap};
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::Path,
    sync::Arc,
};

struct RoofTrace {
    file: std::io::BufWriter<std::fs::File>,
    start: u64,
    end: u64,
}
thread_local! { static ROOF_TRACE: std::cell::RefCell<Option<RoofTrace>> = const { std::cell::RefCell::new(None) }; }
pub fn open_roof_trace(path: &Path, start: u64, end: u64) -> std::io::Result<()> {
    let mut file = std::io::BufWriter::new(std::fs::File::create_new(path)?);
    writeln!(file, "frame,source,geometry_only,native_rotation,native_u,native_v,geometry_rotation,geometry_u,geometry_v,final_u,final_v,ax,ay,au,av,bx,by,bu,bv")?;
    ROOF_TRACE.with(|t| *t.borrow_mut() = Some(RoofTrace { file, start, end }));
    Ok(())
}
pub fn close_roof_trace() -> std::io::Result<()> {
    ROOF_TRACE.with(|t| t.borrow_mut().take().map_or(Ok(()), |mut t| t.file.flush()))
}
fn trace_roof(
    frame: u64,
    source: u32,
    geometry_only: bool,
    native: Orientation,
    geometry: Option<Orientation>,
    result: Orientation,
    a: [i32; 4],
    b: [i32; 4],
) -> std::io::Result<()> {
    ROOF_TRACE.with(|t| {
        let mut t = t.borrow_mut();
        if let Some(t) = t.as_mut().filter(|t| (t.start..=t.end).contains(&frame)) {
            let g = geometry.unwrap_or(Orientation { rotation: 255, ..Default::default() });
            writeln!(t.file,"{frame},{source:08x},{geometry_only},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                native.rotation,native.mirror_u,native.mirror_v,g.rotation,g.mirror_u,g.mirror_v,
                result.mirror_u,result.mirror_v,a[0],a[1],a[2],a[3],b[0],b[1],b[2],b[3])?;
        }
        Ok(())
    })
}
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
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
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
// The special roof path always uses the primary atlas. Reconstruct its UV
// orientation before guest clipping, which otherwise discards wide-only faces.
// Restrict this to axis-aligned, constant-depth quads with an affine UV basis.
fn special_orientation(vertices: &[[f64; 5]], model_orientation: bool) -> Option<Orientation> {
    if vertices.len() != 4 {
        return None;
    }
    let z = vertices[0][2];
    if z <= 0.0
        || vertices
            .iter()
            .any(|p| p.iter().any(|v| !v.is_finite()) || (p[2] - z).abs() > 1e-6)
    {
        return None;
    }
    let points: Vec<_> = vertices
        .iter()
        .map(|p| [p[0] / z, -p[1] / z, p[3], p[4]])
        .collect();
    let [a, b, c] = [points[0], points[1], points[2]];
    let [dx1, dy1, dx2, dy2] = [b[0] - a[0], b[1] - a[1], c[0] - a[0], c[1] - a[1]];
    let det = dx1 * dy2 - dx2 * dy1;
    if det.abs() < 1e-10 {
        return None;
    }
    let gradient = |k: usize| {
        let (d1, d2) = (b[k] - a[k], c[k] - a[k]);
        [(d1 * dy2 - d2 * dy1) / det, (dx1 * d2 - dx2 * d1) / det]
    };
    let (u, v) = (gradient(2), gradient(3));
    for (k, g) in [(2, u), (3, v)] {
        let p = points[3];
        if (a[k] + g[0] * (p[0] - a[0]) + g[1] * (p[1] - a[1]) - p[k]).abs() > 1e-5 {
            return None;
        }
    }
    let rotation = if v[0].abs() > u[0].abs() {
        if model_orientation {
            2
        } else {
            1
        }
    } else {
        0
    };
    let (horizontal, vertical) = match rotation {
        1 => ([-v[0], -v[1]], u),
        2 => (v, [-u[0], -u[1]]),
        _ => (u, v),
    };
    if horizontal[0].abs() < 1e-8
        || vertical[1].abs() < 1e-8
        || horizontal[1].abs() > horizontal[0].abs() * 1e-6
        || vertical[0].abs() > vertical[1].abs() * 1e-6
    {
        return None;
    }
    Some(Orientation {
        rotation,
        mirror_u: horizontal[0] < 0.0,
        mirror_v: vertical[1] < 0.0,
    })
}
fn stable_roof_orientation(
    native: Orientation,
    geometry: Option<Orientation>,
    a: [i32; 4],
    b: [i32; 4],
) -> Orientation {
    let Some(geometry) = geometry.filter(|g| g.rotation == native.rotation) else {
        return native;
    };
    let mut result = native;
    // GBA rounds clipped diagonal endpoints to the same column/row. Its
    // strict comparisons then lose the flip, although a wide view still sees
    // the rest of the roof. Use the non-degenerate source basis on that axis.
    let rotation = Orientation {
        rotation: native.rotation,
        ..Default::default()
    };
    let uv = |p: [i32; 4]| rotation.apply([p[2].clamp(0, 128) as f64, p[3].clamp(0, 128) as f64]);
    let (start, end) = (uv(a), uv(b));
    // Quantized UVs can collapse even when the screen endpoints differ.
    if a[0] == b[0] || start[0] == end[0] {
        result.mirror_u = geometry.mirror_u;
    }
    if a[1] == b[1] || start[1] == end[1] {
        result.mirror_v = geometry.mirror_v;
    }
    result
}
fn merge_texture<K: Eq + std::hash::Hash>(
    bank: &mut HashMap<K, Option<Arc<Vec<u8>>>>,
    key: K,
    texture: &Option<Arc<Vec<u8>>>,
) {
    match bank.entry(key) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(texture.clone());
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            let conflict = match (entry.get(), texture) {
                (Some(old), Some(new)) => {
                    !Arc::ptr_eq(old, new)
                        && (1..127).any(|v| (1..127).any(|u| old[v * 128 + u] != new[v * 128 + u]))
                }
                (_, None) => true,
                _ => false,
            };
            if conflict {
                entry.insert(None);
            }
        }
    }
}
fn remember_texture<K: Copy + Eq + std::hash::Hash>(
    bank: &mut HashMap<K, Option<Arc<Vec<u8>>>>,
    order: &mut std::collections::VecDeque<K>,
    key: K,
    texture: &Option<Arc<Vec<u8>>>,
    limit: usize,
) {
    if !bank.contains_key(&key) {
        while order.len() >= limit {
            if let Some(old) = order.pop_front() {
                bank.remove(&old);
            }
        }
        order.push_back(key);
    }
    merge_texture(bank, key, texture);
}
fn atlas(
    m: &Machine,
    cache: &mut HashMap<(u32, usize), Arc<Vec<u8>>>,
    address: u32,
    size: usize,
) -> Result<Arc<Vec<u8>>, String> {
    if address >> 24 == 8 {
        if let Some(data) = cache.get(&(address, size)) {
            return Ok(data.clone());
        }
        let data = Arc::new(bytes(m, address, size)?.to_vec());
        cache.insert((address, size), data.clone());
        Ok(data)
    } else {
        Ok(Arc::new(bytes(m, address, size)?.to_vec()))
    }
}
fn canonical_atlas(
    cache: &mut HashMap<(u32, Orientation), Arc<Vec<u8>>>,
    address: u32,
    transform: Orientation,
    texture: &Arc<Vec<u8>>,
) -> Arc<Vec<u8>> {
    if address >> 24 == 8 {
        if let Some(data) = cache.get(&(address, transform)) {
            return data.clone();
        }
    }
    let canonical: Arc<Vec<u8>> = Arc::new(
        (0..128 * 128)
            .map(|i| {
                let uv = transform.apply([(i % 128) as f64, (i / 128) as f64]);
                texture[uv[1].floor().clamp(0.0, 127.0) as usize * 128
                    + uv[0].floor().clamp(0.0, 127.0) as usize]
            })
            .collect(),
    );
    if address >> 24 == 8 {
        cache.insert((address, transform), canonical.clone());
    }
    canonical
}
/// One immutable indexed building layer. Palette and sprite animation remain live.
#[derive(Default)]
pub struct LayerCache {
    last: Option<(
        (
            u64,
            [i32; 3],
            usize,
            usize,
            usize,
            crate::render_settings::TextureMapping,
        ),
        Arc<Vec<u8>>,
    )>,
}
impl LayerCache {
    pub fn render(
        &mut self,
        capture: &Capture,
        scene: u64,
        delta: [i32; 3],
        view: crate::viewport::View,
        mapping: crate::render_settings::TextureMapping,
    ) -> Result<Arc<Vec<u8>>, String> {
        let key = (scene, delta, view.width, view.height, view.scale, mapping);
        if let Some((old, layer)) = &self.last {
            if *old == key {
                return Ok(layer.clone());
            }
        }
        let layer = Arc::new(capture.live_layer_view(delta, view, mapping)?);
        self.last = Some((key, layer.clone()));
        Ok(layer)
    }
}
type MaterialKey = (u32, u32, bool, bool);
#[derive(Default)]
pub struct MaterialBank {
    face_order: std::collections::VecDeque<(MaterialKey, u32)>,
    material_order: std::collections::VecDeque<MaterialKey>,
    faces: HashMap<(MaterialKey, u32), Option<Arc<Vec<u8>>>>,
    canonical: HashMap<MaterialKey, Option<Arc<Vec<u8>>>>,
}
impl MaterialBank {
    pub fn restore(&mut self, capture: &mut Capture) -> usize {
        if !capture.wide {
            return 0;
        }
        for (&key, texture) in &capture.canonical {
            // Only immutable cartridge atlases can be reused across captures.
            if key.0 >> 24 != 8 || (key.1 != 0 && key.1 >> 24 != 8) {
                continue;
            }
            remember_texture(
                &mut self.canonical,
                &mut self.material_order,
                key,
                texture,
                128,
            );
        }
        for (&(key, source), texture) in &capture.face_canonical {
            if key.0 >> 24 == 8 && (key.1 == 0 || key.1 >> 24 == 8) {
                remember_texture(
                    &mut self.faces,
                    &mut self.face_order,
                    (key, source),
                    texture,
                    512,
                );
            }
        }
        let mut count = 0;
        for (i, face) in capture.faces.iter_mut().enumerate() {
            if face.input.size != 128
                || face.input.uv.is_empty()
                || (!face.texture.is_empty() && !capture.inferred.contains(&i))
            {
                continue;
            }
            if let Some(key) = capture.material_keys.get(&i) {
                let face_key = (*key, face.input.source);
                if matches!(capture.face_canonical.get(&face_key), Some(None)) {
                    continue;
                }
                let texture = self
                    .faces
                    .get(&face_key)
                    .or_else(|| self.canonical.get(key));
                if let Some(Some(texture)) = texture {
                    face.texture = texture.clone();
                    for (vertex, uv) in face.vertices.iter_mut().zip(&face.input.uv) {
                        vertex[3] = uv[0];
                        vertex[4] = uv[1];
                    }
                    capture.inferred.remove(&i);
                    count += 1;
                }
            }
        }
        count
    }
}
#[derive(Clone, Default)]
pub struct Capture {
    atlases: HashMap<(u32, usize), Arc<Vec<u8>>>,
    canonical_atlases: HashMap<(u32, Orientation), Arc<Vec<u8>>>,
    faces: Vec<Face>,
    group_start: usize,
    descriptor: u32,
    orientation: bool,
    special: bool,
    quads: HashMap<u32, usize>,
    rows: Vec<String>,
    palette: Vec<u8>,
    pub wide: bool,
    pub geometry_only: bool,
    pub side_faces: usize,
    inferred: HashSet<usize>,
    material_keys: HashMap<usize, MaterialKey>,
    canonical: HashMap<MaterialKey, Option<Arc<Vec<u8>>>>,
    face_canonical: HashMap<(MaterialKey, u32), Option<Arc<Vec<u8>>>>,
    offscreen_special: HashMap<usize, (Arc<Vec<u8>>, Orientation)>,
}
fn same_face(a: &Face, b: &Face) -> bool {
    a.input.source == b.input.source
        && a.vertices.len() == b.vertices.len()
        && a.vertices.iter().zip(&b.vertices).all(|(a, b)| {
            a[..3]
                .iter()
                .zip(&b[..3])
                .all(|(x, y)| (x - y).abs() < 1e-7)
        })
}
impl Capture {
    pub fn append_missing_geometry(&mut self, side: &Self) -> usize {
        // The side pass preserves the native painter order. Insert missing faces
        // around shared anchors rather than painting every edge building last.
        let mut original = std::mem::take(&mut self.faces);
        let keys = std::mem::take(&mut self.material_keys);
        let mut inferred = std::mem::take(&mut self.inferred);
        let mut ordered: Vec<(bool, usize)> = Vec::new();
        let mut pending = Vec::new();
        let mut cursor = 0;
        let mut count = 0;
        for (index, face) in side.faces.iter().enumerate() {
            let anchor = original.iter().position(|f| same_face(f, face));
            if let Some(anchor) = anchor {
                // A shared face can be clipped out of the guest material pass
                // while its precise geometry still covers host pixels. Preserve
                // the original painter anchor, but fill a missing atlas/UV pair
                // from the same face and material in the wider pass.
                let target = &mut original[anchor];
                if self.wide
                    && target.texture.is_empty()
                    && !face.texture.is_empty()
                    && target.input.size == face.input.size
                    && target.input.color == face.input.color
                    && target.input.uv == face.input.uv
                    && keys
                        .get(&anchor)
                        .is_some_and(|key| side.material_keys.get(&index) == Some(key))
                {
                    target.texture = face.texture.clone();
                    for (vertex, source) in target.vertices.iter_mut().zip(&face.vertices) {
                        vertex[3..5].copy_from_slice(&source[3..5]);
                    }
                    if side.inferred.contains(&index) {
                        inferred.insert(anchor);
                    }
                }
                if anchor >= cursor {
                    ordered.extend((cursor..anchor).map(|i| (false, i)));
                    ordered.extend(pending.drain(..).map(|i| (true, i)));
                    ordered.push((false, anchor));
                    cursor = anchor + 1;
                }
            } else {
                pending.push(index);
                count += 1;
            }
        }
        ordered.extend((cursor..original.len()).map(|i| (false, i)));
        ordered.extend(pending.into_iter().map(|i| (true, i)));
        for (from_side, index) in ordered {
            let (face, key, provisional) = if from_side {
                (
                    &side.faces[index],
                    side.material_keys.get(&index),
                    side.inferred.contains(&index),
                )
            } else {
                (
                    &original[index],
                    keys.get(&index),
                    inferred.contains(&index),
                )
            };
            let target = self.faces.len();
            self.faces.push(face.clone());
            if let Some(key) = key {
                self.material_keys.insert(target, *key);
            }
            if provisional {
                self.inferred.insert(target);
            }
        }
        self.side_faces += count;
        count
    }
    pub fn complete_offscreen(&mut self) -> usize {
        if !self.wide {
            return 0;
        }
        let mut count = 0;
        for (i, face) in self.faces.iter_mut().enumerate() {
            if face.texture.is_empty() && face.input.size == 128 && !face.input.uv.is_empty() {
                if let Some((texture, transform)) = self.offscreen_special.get(&i) {
                    face.texture = texture.clone();
                    self.inferred.insert(i);
                    for vertex in &mut face.vertices {
                        let uv = transform.apply([vertex[3], vertex[4]]);
                        vertex[3] = uv[0];
                        vertex[4] = uv[1];
                    }
                    count += 1;
                    continue;
                }
                if let Some(Some(texture)) = self
                    .material_keys
                    .get(&i)
                    .and_then(|k| self.canonical.get(k))
                {
                    face.texture = texture.clone();
                    count += 1;
                }
            }
        }
        count
    }
    #[cfg(test)]
    pub fn supports_camera(&self, delta: [i32; 3], scale: usize) -> bool {
        self.supports_view(delta, crate::viewport::View::new(scale, Default::default()))
    }
    pub fn supports_view(&self, delta: [i32; 3], view: crate::viewport::View) -> bool {
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
                !polygon_has_samples_view(&clip_horizontal(vertices, view.extent()), view)
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
        self.live_layer_view(
            delta,
            crate::viewport::View::new(scale, Default::default()),
            mapping,
        )
    }
    pub fn live_layer_view(
        &self,
        delta: [i32; 3],
        view: crate::viewport::View,
        mapping: crate::render_settings::TextureMapping,
    ) -> Result<Vec<u8>, String> {
        let scale = view.scale;
        // A model fully culled by the guest can omit its draw-batch call.
        // Only a visible textured face needs a captured texture here. Flat
        // faces use the scene palette supplied by the live compositor.
        let mut layer = vec![0; view.width * view.height];
        for (face_index, face) in self.faces.iter().enumerate() {
            let mut vertices = face.vertices.clone();
            for v in &mut vertices {
                v[0] -= delta[0] as f64 / 65536.0;
                v[1] += delta[1] as f64 / 65536.0;
                v[2] += delta[2] as f64 / 65536.0;
            }
            let vertices = clip_horizontal(vertices, view.extent());
            if vertices.len() < 3 {
                continue;
            }
            if !face.input.uv.is_empty() && face.texture.is_empty() {
                if polygon_has_samples_view(&vertices, view) {
                    return Err(format!("Visible face has no captured material: source={:08x} size={} key={:?} canonical={:?}",face.input.source,face.input.size,self.material_keys.get(&face_index),self.material_keys.get(&face_index).and_then(|key|self.canonical.get(key)).map(|value|value.is_some())));
                }
                continue;
            }
            let v: Vec<_> = vertices
                .iter()
                .map(|v| {
                    [
                        120.0 + view.offset() + 118.0 * v[0] / v[2],
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
            let texture = if m.cpu.regs[1] == 0 {
                Arc::new(Vec::new())
            } else {
                atlas(m, &mut self.atlases, m.cpu.regs[1], 4096)?
            };
            for i in self.group_start..self.faces.len() {
                if !self.wide {
                    break;
                }
                if self.faces[i].input.size == 128 && !self.faces[i].input.uv.is_empty() {
                    if self.special || self.geometry_only {
                        if let Some(mut transform) =
                            special_orientation(&self.faces[i].vertices, self.orientation)
                        {
                            let alternate = if self.special {
                                false
                            } else {
                                transform.mirror_u = false;
                                transform.mirror_v = false;
                                if transform.rotation != 0 {
                                    !self.orientation
                                } else {
                                    self.orientation
                                }
                            };
                            let pointer =
                                word(m, self.descriptor + if alternate { 4 } else { 0 })? as u32;
                            if pointer >> 24 == 8 {
                                let atlas = atlas(m, &mut self.atlases, pointer, 128 * 128)?;
                                self.offscreen_special.insert(i, (atlas, transform));
                            }
                        }
                    }
                    self.material_keys.insert(
                        i,
                        (
                            word(m, self.descriptor)? as u32,
                            word(m, self.descriptor + 4)? as u32,
                            self.orientation,
                            self.special,
                        ),
                    );
                }
            }
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
            let guest_transform = transform;
            if self.special {
                let geometry = special_orientation(&face.vertices, self.orientation);
                transform = stable_roof_orientation(transform, geometry, a, b);
                trace_roof(
                    m.bus.frames,
                    face.input.source,
                    self.geometry_only,
                    guest_transform,
                    geometry,
                    transform,
                    a,
                    b,
                )
                .map_err(|e| e.to_string())?;
            }
            face.texture = atlas(m, &mut self.atlases, full, 128 * 128)?;
            if let Some(&key) = self.material_keys.get(&index) {
                let canonical = Some(canonical_atlas(
                    &mut self.canonical_atlases,
                    full,
                    transform,
                    &face.texture,
                ));
                merge_texture(&mut self.canonical, key, &canonical);
                merge_texture(
                    &mut self.face_canonical,
                    (key, face.input.source),
                    &canonical,
                );
            }
            for vertex in &mut face.vertices {
                let p = transform.apply([vertex[3], vertex[4]]);
                vertex[3] = p[0];
                vertex[4] = p[1];
            }
            self.rows.push(format!("{index},{descriptor:08x},{selected:08x},{full:08x},{mip},{},{},{},{alternate},{},{},{},{}",guest_transform.rotation,guest_transform.mirror_u,guest_transform.mirror_v,checked_uv[0],checked_uv[1],checked_uv[2],checked_uv[3]));
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
fn polygon_has_samples_view(vertices: &[[f64; 5]], view: crate::viewport::View) -> bool {
    let scale = view.scale;
    if vertices.len() < 3 {
        return false;
    }
    let project = |v: &[f64; 5]| {
        [
            120.0 + view.offset() + 118.0 * v[0] / v[2],
            120.0 - 118.0 * v[1] / v[2],
        ]
    };
    (1..vertices.len() - 1).any(|i| {
        triangle_has_samples_width(
            [
                project(&vertices[0]),
                project(&vertices[i]),
                project(&vertices[i + 1]),
            ],
            scale,
            view.width,
        )
    })
}
#[cfg(test)]
fn triangle_has_samples(v: [[f64; 2]; 3], scale: usize) -> bool {
    triangle_has_samples_width(v, scale, 240 * scale)
}
fn triangle_has_samples_width(v: [[f64; 2]; 3], scale: usize, width: usize) -> bool {
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
        for x in lo(0)..hi(0, width) {
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
fn clip(vertices: Vec<[f64; 5]>) -> Vec<[f64; 5]> {
    clip_horizontal(vertices, 1.0)
}
fn clip_horizontal(mut vertices: Vec<[f64; 5]>, extent: f64) -> Vec<[f64; 5]> {
    for plane in 0..5 {
        let distance = |v: [f64; 5]| match plane {
            0 => v[2] - 32.0,
            1 => v[2] * extent + v[0],
            2 => v[2] * extent - v[0],
            3 => v[2] + v[1],
            // Screen y = 120 - 118*y/z: the top edge is y/z=120/118,
            // not 1.0 (which leaves two native rows of missing buildings).
            _ => v[2] * (120.0 / 118.0) - v[1],
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
    let width = image.len() / (160 * scale);
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
            let (mut left, mut right) = (0.0f64, width as f64 / scale as f64);
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
    #[test]
    fn clipped_roof_keeps_flip_when_uv_quantizes_before_screen_position() {
        let native = Orientation {
            rotation: 0,
            mirror_u: false,
            mirror_v: true,
        };
        let geometry = Orientation {
            rotation: 0,
            mirror_u: true,
            mirror_v: true,
        };
        // Captured at frame 19202, source 08da934c: two screen columns, one texel.
        let fixed = stable_roof_orientation(native, Some(geometry), [3, 2, 0, 118], [2, 108, 0, 0]);
        assert!(fixed.mirror_u && fixed.mirror_v);
        let rotated = Orientation {
            rotation: 1,
            mirror_u: false,
            mirror_v: false,
        };
        let geometry = Orientation {
            rotation: 1,
            mirror_u: true,
            mirror_v: true,
        };
        let fixed =
            stable_roof_orientation(rotated, Some(geometry), [10, 20, 0, -1], [11, 21, 1, 0]);
        assert!(fixed.mirror_u); // Rotation maps the clamped V coordinate to U.
        assert!(!fixed.mirror_v); // Non-degenerate axis retains the verified guest sign.
    }
    #[test]
    fn atlas_sharing_keeps_ram_live_and_orientation_variants_independent() {
        let mut m = Machine::new(vec![7; 128 * 128]);
        let mut cache = HashMap::new();
        let a = atlas(&m, &mut cache, 0x08000000, 128 * 128).unwrap();
        let b = atlas(&m, &mut cache, 0x08000000, 128 * 128).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        let ram = atlas(&m, &mut cache, 0x02000100, 4096).unwrap();
        m.bus.ewram[0x100] = 3;
        let changed = atlas(&m, &mut cache, 0x02000100, 4096).unwrap();
        assert_eq!(ram[0], 0);
        assert_eq!(changed[0], 3);
        let texture = Arc::new((0..128 * 128).map(|i| (i % 251) as u8).collect());
        let mut transformed = HashMap::new();
        let first = canonical_atlas(
            &mut transformed,
            0x08000000,
            Orientation::default(),
            &texture,
        );
        let second = canonical_atlas(
            &mut transformed,
            0x08000000,
            Orientation::default(),
            &texture,
        );
        assert!(Arc::ptr_eq(&first, &second));
        let flipped = canonical_atlas(
            &mut transformed,
            0x08000000,
            Orientation {
                mirror_u: true,
                ..Default::default()
            },
            &texture,
        );
        assert_ne!(first, flipped);
    }
    #[test]
    fn indexed_layer_cache_tracks_scene_camera_view_and_mapping() {
        let mut capture = Capture::default();
        capture.push(
            vec![
                [-50., 50., 100.],
                [50., 50., 100.],
                [50., -50., 100.],
                [-50., -50., 100.],
            ],
            Input {
                source: 0x08000000,
                color: 7,
                size: 128,
                uv: Vec::new(),
            },
        );
        let mut cache = LayerCache::default();
        let view = crate::viewport::View::new(3, crate::viewport::Aspect::Wide);
        let mapping = crate::render_settings::TextureMapping::Perspective;
        let first = cache.render(&capture, 1, [0; 3], view, mapping).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &cache.render(&capture, 1, [0; 3], view, mapping).unwrap()
        ));
        for (scene, delta, view, mapping) in [
            (2, [0; 3], view, mapping),
            (2, [65536, 0, 0], view, mapping),
            (
                2,
                [65536, 0, 0],
                crate::viewport::View::new(3, crate::viewport::Aspect::Original),
                mapping,
            ),
            (
                2,
                [65536, 0, 0],
                view,
                crate::render_settings::TextureMapping::Affine,
            ),
        ] {
            let expected = capture.live_layer_view(delta, view, mapping).unwrap();
            let actual = cache.render(&capture, scene, delta, view, mapping).unwrap();
            assert_eq!(*actual, expected);
            assert!(!Arc::ptr_eq(&first, &actual));
        }
    }
    #[test]
    fn clipped_roof_keeps_orientation_when_guest_diagonal_collapses_to_one_column() {
        let native = Orientation {
            rotation: 0,
            mirror_u: false,
            mirror_v: true,
        };
        let geometry = Orientation {
            rotation: 0,
            mirror_u: true,
            mirror_v: true,
        };
        let stable =
            stable_roof_orientation(native, Some(geometry), [239, 0, 0, 0], [239, 40, 100, 100]);
        assert!(stable.mirror_u && stable.mirror_v);
        let valid =
            stable_roof_orientation(native, Some(geometry), [230, 0, 0, 0], [239, 40, 100, 100]);
        assert!(!valid.mirror_u);
        assert_eq!(
            stable_roof_orientation(native, None, [239; 4], [239; 4]).mirror_u,
            false
        );
    }
    #[test]
    fn side_building_is_inserted_before_the_nearer_anchor_in_painter_order() {
        let near_input = Input {
            source: 0x08000100,
            color: 3,
            size: 64,
            uv: Vec::new(),
        };
        let near_vertices = vec![
            [-120., 100., 100.],
            [-80., 100., 100.],
            [-80., 60., 100.],
            [-120., 60., 100.],
        ];
        let mut main = Capture::default();
        main.wide = true;
        main.push(near_vertices.clone(), near_input.clone());
        let mut side = Capture::default();
        side.push(
            vec![
                [-180., 120., 120.],
                [-100., 120., 120.],
                [-100., 60., 120.],
                [-180., 60., 120.],
            ],
            Input {
                source: 0x08000200,
                color: 5,
                size: 64,
                uv: Vec::new(),
            },
        );
        side.push(near_vertices, near_input);
        assert_eq!(main.append_missing_geometry(&side), 1);
        let view = crate::viewport::View::new(1, crate::viewport::Aspect::Wide);
        let layer = main
            .live_layer_view(
                [0; 3],
                view,
                crate::render_settings::TextureMapping::Perspective,
            )
            .unwrap();
        assert_eq!(layer[30 * view.width + view.padding + 10], 3);
        assert_eq!(main.faces[0].input.source, 0x08000200);
        assert_eq!(main.faces[1].input.source, 0x08000100);
    }
    #[test]
    fn inferred_roof_uses_the_captured_mapping_even_after_side_faces_are_inserted() {
        let key = (0x08001000, 0x08001000, false, true);
        let mut main = Capture::default();
        main.wide = true;
        main.push(
            vec![[0., 0., 100.]; 3],
            Input {
                source: 0x08000100,
                color: 1,
                size: 64,
                uv: Vec::new(),
            },
        );
        let mut side = Capture::default();
        side.wide = true;
        side.push(
            vec![[0., 0., 100.]; 4],
            Input {
                source: 0x08000200,
                color: 1,
                size: 128,
                uv: vec![[12., 34.]; 4],
            },
        );
        side.faces[0].texture = Arc::new(vec![8; 16384]);
        side.faces[0].vertices[0][3] = 99.;
        side.material_keys.insert(0, key);
        side.inferred.insert(0);
        main.append_missing_geometry(&side);
        let index = main
            .faces
            .iter()
            .position(|f| f.input.source == 0x08000200)
            .unwrap();
        assert_eq!(main.material_keys.get(&index), Some(&key));
        let mut bank = MaterialBank::default();
        bank.faces
            .insert((key, 0x08000200), Some(Arc::new(vec![7; 16384])));
        assert_eq!(bank.restore(&mut main), 1);
        assert_eq!(main.faces[index].texture[0], 7);
        assert_eq!(&main.faces[index].vertices[0][3..], &[12., 34.]);
        assert!(!main.inferred.contains(&index));
    }

    #[test]
    fn wide_geometry_merge_fills_a_shared_clipped_material_with_its_uv_pair() {
        let key = (0x08420668, 0x08424668, true, false);
        let mut main = Capture::default();
        main.wide = true;
        main.push(
            vec![
                [-33., -68., 180.],
                [33., -68., 180.],
                [33., 69., 180.],
                [-33., 69., 180.],
            ],
            Input {
                source: 0x08dbfc4c,
                color: 7,
                size: 128,
                uv: vec![[127., 0.], [63., 0.], [63., 127.], [127., 127.]],
            },
        );
        main.material_keys.insert(0, key);
        let mut side = main.clone();
        side.faces[0].texture = Arc::new(vec![9; 16384]);
        for vertex in &mut side.faces[0].vertices {
            let [u, v] = [vertex[3], vertex[4]];
            vertex[3] = 127. - v;
            vertex[4] = u;
        }
        side.inferred.insert(0);
        let mut wrong_material = side.clone();
        wrong_material
            .material_keys
            .insert(0, (key.0, key.1, false, false));
        assert_eq!(main.append_missing_geometry(&wrong_material), 0);
        assert!(main.faces[0].texture.is_empty());
        assert_eq!(main.append_missing_geometry(&side), 0);
        assert_eq!(main.faces.len(), 1);
        assert_eq!(main.faces[0].texture[0], 9);
        assert_eq!(main.faces[0].vertices, side.faces[0].vertices);
        assert!(main.inferred.contains(&0));
        let view = crate::viewport::View::new(3, crate::viewport::Aspect::Wide);
        assert!(main
            .live_layer_view(
                [0; 3],
                view,
                crate::render_settings::TextureMapping::Perspective
            )
            .is_ok());
    }

    #[test]
    fn wide_geometry_merge_keeps_central_texture_and_adds_distinct_world_instances_once() {
        let input = Input {
            source: 0x08000100,
            color: 1,
            size: 64,
            uv: vec![[0., 0.]; 3],
        };
        let vertices = vec![[0., 0., 100.], [10., 0., 100.], [0., 10., 100.]];
        let mut main = Capture::default();
        main.push(vertices.clone(), input.clone());
        main.faces[0].texture = Arc::new(vec![7; 4096]);
        let mut side = Capture::default();
        side.push(vertices, input.clone());
        side.faces[0].texture = Arc::new(vec![9; 4096]);
        side.push(
            vec![[150., 0., 100.], [160., 0., 100.], [150., 10., 100.]],
            input,
        );
        side.faces[1].texture = Arc::new(vec![11; 4096]);
        assert_eq!(main.append_missing_geometry(&side), 1);
        assert_eq!(main.faces[0].texture[0], 7);
        assert_eq!(main.faces[1].texture[0], 11);
        assert_eq!(main.append_missing_geometry(&side), 0);
        assert_eq!(main.side_faces, 1);
    }

    #[test]
    fn roof_uv_basis_handles_all_axis_directions_and_rejects_tilted_faces() {
        for swapped in [false, true] {
            for flip_u in [false, true] {
                for flip_v in [false, true] {
                    for alternate in [false, true] {
                        let vertices: Vec<_> = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]]
                            .iter()
                            .map(|&[x, y]| {
                                let (mut u, mut v) = if swapped { (y, x) } else { (x, y) };
                                if flip_u {
                                    u = 1. - u;
                                }
                                if flip_v {
                                    v = 1. - v;
                                }
                                [10. * x, -10. * y, 105., 128. * u, 128. * v]
                            })
                            .collect();
                        let orientation = super::special_orientation(&vertices, alternate).unwrap();
                        let mapped: Vec<_> = vertices
                            .iter()
                            .map(|p| orientation.apply([p[3], p[4]]))
                            .collect();
                        assert_eq!(mapped, vec![[0., 0.], [128., 0.], [128., 128.], [0., 128.]]);
                        let mut tilted = vertices.clone();
                        tilted[3][2] += 1.;
                        assert!(super::special_orientation(&tilted, alternate).is_none());
                        let mut wrong_uv = vertices.clone();
                        wrong_uv[3][3] += 1.;
                        assert!(super::special_orientation(&wrong_uv, alternate).is_none());
                    }
                }
            }
        }
        assert!(super::special_orientation(&[[0.; 5]; 4], false).is_none());
    }
    #[test]
    fn offscreen_roof_uses_rom_atlas_and_preserves_visible_guest_face() {
        let mut machine = Machine::new(vec![13; 128 * 128]);
        machine.cpu.regs[0] = 0x02000000;
        machine.cpu.regs[3] = 1;
        machine.bus.ewram[..4].copy_from_slice(&0x08000000u32.to_le_bytes());
        let mut capture = Capture::default();
        capture.wide = true;
        for source in [0x08000020, 0x08000040] {
            capture.push(
                vec![
                    [0., 0., 105.],
                    [10., 0., 105.],
                    [10., -10., 105.],
                    [0., -10., 105.],
                ],
                Input {
                    source,
                    color: 1,
                    size: 128,
                    uv: vec![[128., 128.], [0., 128.], [0., 0.], [128., 0.]],
                },
            );
        }
        let visible = Arc::new(vec![7; 128 * 128]);
        capture.faces[1].texture = visible.clone();
        capture.observe(&machine, 0x0800db65).unwrap();
        assert!(capture.faces[0].texture.is_empty());
        assert_eq!(capture.complete_offscreen(), 1);
        assert_eq!(capture.faces[0].texture[0], 13);
        assert_eq!(&capture.faces[0].vertices[0][3..], &[0., 0.]);
        assert!(Arc::ptr_eq(&capture.faces[1].texture, &visible));
        assert_eq!(&capture.faces[1].vertices[0][3..], &[128., 128.]);
        assert_eq!(capture.complete_offscreen(), 0);
    }

    use super::*;
    #[test]
    fn building_faces_reach_the_top_row_at_both_aspects_and_scales() {
        let mut capture = Capture::default();
        capture.push(
            vec![
                [-80.0, 130.0, 100.0],
                [-40.0, 130.0, 100.0],
                [-40.0, 90.0, 100.0],
                [-80.0, 90.0, 100.0],
            ],
            Input {
                source: 0,
                color: 9,
                size: 64,
                uv: Vec::new(),
            },
        );
        for aspect in [
            crate::viewport::Aspect::Original,
            crate::viewport::Aspect::Wide,
        ] {
            for scale in [1, 3] {
                let view = crate::viewport::View::new(scale, aspect);
                let layer = capture
                    .live_layer_view(
                        [0; 3],
                        view,
                        crate::render_settings::TextureMapping::Perspective,
                    )
                    .unwrap();
                for y in 0..2 * scale {
                    assert_eq!(layer[y * view.width + 40 * scale + view.padding], 9);
                }
            }
        }
    }
    #[test]
    fn wide_visibility_checks_include_side_faces_and_cache_is_bounded() {
        let mut capture = Capture::default();
        capture.push(
            vec![
                [110.0, 0.0, 100.0],
                [115.0, 0.0, 100.0],
                [110.0, 5.0, 100.0],
            ],
            Input {
                source: 0x08000100,
                color: 1,
                size: 128,
                uv: vec![[0.0, 0.0]; 3],
            },
        );
        let view = crate::viewport::View::new(3, crate::viewport::Aspect::Wide);
        assert!(capture.supports_camera([0; 3], 3));
        assert!(!capture.supports_view([0; 3], view));
        assert!(capture.supports_view([-30 * 65536, 0, 0], view));
        let mut bank = HashMap::new();
        let mut order = std::collections::VecDeque::new();
        for i in 0..300 {
            remember_texture(&mut bank, &mut order, i, &None, 128);
        }
        assert_eq!(bank.len(), 128);
        assert!(!bank.contains_key(&0));
        assert!(bank.contains_key(&299));
    }
    #[test]
    fn material_bank_recovers_culled_faces_without_reusing_mutable_atlases() {
        let mut known = Capture::default();
        known.wide = true;
        let key = (0x08000400, 0x08008000, false, false);
        known
            .canonical
            .insert(key, Some(Arc::new(vec![9; 128 * 128])));
        let mut bank = MaterialBank::default();
        bank.restore(&mut known);
        let mut next = Capture::default();
        next.wide = true;
        next.push(
            vec![[0.0, 0.0, 100.0]; 3],
            Input {
                source: 0x08000200,
                color: 1,
                size: 128,
                uv: vec![[0.0, 0.0]; 3],
            },
        );
        next.material_keys.insert(0, key);
        assert_eq!(bank.restore(&mut next), 1);
        assert_eq!(next.faces[0].texture[0], 9);
        next.faces[0].texture = Arc::new(Vec::new());
        next.material_keys.insert(0, (0x02000400, 0, false, false));
        assert_eq!(bank.restore(&mut next), 0);
        next.material_keys.insert(0, key);
        next.canonical.insert(key, None);
        assert_eq!(bank.restore(&mut next), 0);
        known
            .face_canonical
            .insert((key, 0x08000200), Some(Arc::new(vec![5; 128 * 128])));
        bank.restore(&mut known);
        assert_eq!(bank.restore(&mut next), 1);
        assert_eq!(next.faces[0].texture[0], 5);
    }
    #[test]
    fn offscreen_materials_require_the_same_batch_and_leave_visible_faces_untouched() {
        let mut capture = Capture::default();
        capture.wide = true;
        for source in [0x08000100, 0x08000200, 0x08000300] {
            capture.push(
                vec![[0.0, 0.0, 100.0]; 3],
                Input {
                    source,
                    color: 1,
                    size: 128,
                    uv: vec![[0.0, 0.0]; 3],
                },
            );
        }
        let visible = Arc::new(vec![7; 128 * 128]);
        capture.faces[0].texture = visible.clone();
        capture
            .material_keys
            .insert(0, (0x08000400, 0x08008000, false, false));
        capture
            .material_keys
            .insert(1, (0x08000400, 0x08008000, false, false));
        capture
            .material_keys
            .insert(2, (0x08000400, 0x08008000, true, false));
        let texture = Arc::new(vec![8; 128 * 128]);
        capture.canonical.insert(
            (0x08000400, 0x08008000, false, false),
            Some(texture.clone()),
        );
        assert_eq!(capture.complete_offscreen(), 1);
        assert!(Arc::ptr_eq(&capture.faces[0].texture, &visible));
        assert!(Arc::ptr_eq(&capture.faces[1].texture, &texture));
        assert!(capture.faces[2].texture.is_empty());
    }
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
