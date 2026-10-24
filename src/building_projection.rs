//! Read-only USA building transform/projection checks and fractional outlines.
//! Wide model selection runs in an isolated native geometry pass.
use gba_core::{Machine, MemMap};
use std::{collections::HashMap, io::Write, path::Path};
pub(crate) fn bytes(m: &Machine, addr: u32, n: usize) -> Result<&[u8], String> {
    let (mask, data) = match addr >> 24 {
        2 => (0x3ffff, &m.bus.ewram),
        3 => (0x7fff, &m.bus.iwram),
        8 => (0x1ffffff, &m.bus.rom),
        _ => return Err(format!("Building address region {addr:08x}")),
    };
    let offset = (addr & mask) as usize;
    data.get(offset..offset + n)
        .ok_or_else(|| format!("Building address bounds {addr:08x}"))
}
pub(crate) fn word(m: &Machine, addr: u32) -> Result<i32, String> {
    Ok(i32::from_le_bytes(bytes(m, addr, 4)?.try_into().unwrap()))
}
#[derive(Clone, Copy)]
struct Vertex {
    integer: [i32; 3],
    precise: [f64; 3],
    uv64: Option<[i32; 2]>,
}
struct Transform {
    target: u32,
    dest: u32,
    vertices: Vec<Vertex>,
    material: crate::building_material::Input,
}
struct Projection {
    target: u32,
    expected: [i32; 2],
    precise: Option<[f64; 2]>,
}
#[derive(Default)]
pub struct Observer {
    objects: crate::object_projection::Observer,
    materials: crate::building_material::Capture,
    offsets: [Option<i32>; 3],
    vertices: HashMap<u32, Vertex>,
    pending: Option<Transform>,
    projection: Option<Projection>,
    faces: Vec<Vec<[f64; 3]>>,
    vertices_checked: usize,
    projections_checked: usize,
    mapped_projections: usize,
    unmapped_projections: usize,
    uv64_checked: usize,
    rows: Vec<String>,
    camera: Option<[i32; 3]>,
    extend_world: bool,
    extended_materials: Option<crate::building_material::Capture>,
}
fn wide_model_edge_tile(regs: &mut [u32; 16]) {
    let (left, right) = (regs[5] as i32, regs[4] as i32);
    if right < 0 && right >= -32 {
        regs[5] = 0;
        regs[4] = 8;
    } else if left > 240 && left <= 272 {
        regs[5] = 232;
        regs[4] = 240;
    }
}
fn project(v: [f64; 3]) -> [f64; 2] {
    [120.0 + 118.0 * v[0] / v[2], 120.0 - 118.0 * v[1] / v[2]]
}
impl Observer {
    pub fn set_wide(&mut self, wide: bool) {
        self.extend_world = wide;
        self.materials.wide = wide;
        self.objects.wide = wide;
    }

    pub fn side_failures(&self) -> &[String] {
        self.objects.side_failures()
    }
    #[inline]
    pub fn interested(&self, key: u32) -> bool {
        matches!(
            key,
            0x0800d301
                | 0x0800e045
                | 0x0800e04f
                | 0x0800e059
                | 0x030018a4
                | 0x03001968
                | 0x03001720
                | 0x030017f8
        ) || self.pending.as_ref().is_some_and(|p| p.target == key)
            || self.projection.as_ref().is_some_and(|p| p.target == key)
            || crate::object_projection::Observer::interested(key)
            || crate::building_material::Capture::interested(key)
    }
    pub fn into_live(
        mut self,
        source: &MemMap,
    ) -> Result<
        (
            crate::building_material::Capture,
            crate::object_projection::Snapshot,
        ),
        String,
    > {
        if self.pending.is_some() || self.projection.is_some() {
            return Err("Incomplete geometry capture".into());
        }
        crate::ground_projection::validate_live(source)?;
        if source.io[0x10..0x14] != [0; 4] {
            return Err("Unsupported building scroll".into());
        }
        let objects = self.objects.live_snapshot(source)?;
        self.materials.complete_offscreen();
        if let Some(materials) = self.extended_materials.take() {
            self.materials.append_missing_geometry(&materials);
        }
        self.materials.live_layer([0; 2], 1)?;
        Ok((self.materials, objects))
    }
    fn wide_buildings(m: &Machine) -> Result<crate::building_material::Capture, String> {
        let mut fork = m.try_fork_for_research().map_err(str::to_string)?;
        let mut capture = Self::default();
        capture.materials.wide = true;
        capture.materials.geometry_only = true;
        let scale = word(m, 0x03000060)? as f64 / 65536.;
        if scale <= 0. {
            return Err("Invalid wide building scale".into());
        }
        let padding = (crate::viewport::View::new(3, crate::viewport::Aspect::Wide).offset()
            / scale
            * 65536.)
            .ceil() as i32;
        crate::scene_replay::presentation_observe(&mut fork, 0x0800d53f, |fork, key| {
            if key == 0x0800d319 {
                for (offset, delta) in [(0x84, -padding), (0x88, padding)] {
                    let value =
                        i32::from_le_bytes(fork.bus.iwram[offset..offset + 4].try_into().unwrap());
                    fork.bus.iwram[offset..offset + 4]
                        .copy_from_slice(&value.wrapping_add(delta).to_le_bytes());
                }
            }
            // The guest's screen-tile visibility bookkeeping has only 30
            // columns. Map side-only bounds to the nearest edge tile in this
            // copy; actual geometry retains world coordinates for the host clipper.
            if key == 0x0800e6c1 {
                wide_model_edge_tile(&mut fork.cpu.regs);
            }
            if capture.interested(key) {
                capture.observe(fork, key)?;
            }
            // Geometry and atlas pointers are complete at the batch boundary.
            // The host rasterizer consumes them, so guest pixel loops are unnecessary.
            if key == 0x0800db65 {
                fork.cpu.regs[15] = fork.cpu.regs[14] & !1;
            }
            Ok(())
        })?;
        capture.materials.complete_offscreen();
        Ok(capture.materials)
    }
    pub fn observe(&mut self, m: &Machine, key: u32) -> Result<(), String> {
        if key == 0x0800d301 && self.extend_world {
            match Self::wide_buildings(m) {
                Ok(materials) => self.extended_materials = Some(materials),
                Err(reason) => self
                    .objects
                    .record_side_failure(format!("buildings: {reason}")),
            }
        }
        self.objects.observe(m, key)?;
        self.materials.observe(m, key)?;
        if let Some(axis) = [0x0800e045, 0x0800e04f, 0x0800e059]
            .iter()
            .position(|p| *p == key)
        {
            if axis == 0 {
                self.offsets = [None; 3];
                self.vertices.clear();
            }
            let camera = [0x03000014, 0x03000018, 0x0300001c]
                .map(|a| word(m, a))
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;
            let camera: [i32; 3] = camera.try_into().unwrap();
            if self.camera.is_some_and(|c| c != camera) {
                return Err("Building capture camera changed within scene".into());
            }
            self.camera = Some(camera);
            let offset = m.cpu.regs[0] as i32;
            if offset.wrapping_add(m.cpu.regs[1] as i32) != camera[axis] {
                return Err("Building origin/camera subtraction mismatch".into());
            }
            self.offsets[axis] = Some(offset);
        }
        if self.pending.as_ref().is_some_and(|p| p.target == key) {
            let pending = self.pending.take().unwrap();
            for (i, vertex) in pending.vertices.iter().enumerate() {
                let addr = pending.dest + i as u32 * 20;
                let actual = [word(m, addr)?, word(m, addr + 4)?, word(m, addr + 8)?];
                if actual != vertex.integer {
                    return Err(format!(
                        "Building transform mismatch at {addr:08x}: {actual:?} != {:?}",
                        vertex.integer
                    ));
                }
                self.vertices.insert(addr, *vertex);
                self.vertices_checked += 1;
            }
            if m.cpu.regs[0] as i32 <= 0 {
                self.materials.push(
                    pending.vertices.iter().map(|v| v.precise).collect(),
                    pending.material,
                );
                self.faces
                    .push(pending.vertices.iter().map(|v| v.precise).collect());
            }
        }
        if [0x030018a4, 0x03001968].contains(&key) && word(m, key + 4)? as u32 == 0xe8b20038 {
            if self.pending.is_some() {
                return Err("Nested building transform".into());
            }
            // The stack buffer is reused for each face. Clipping a triangle
            // can add a fourth vertex at the preceding quad's old address,
            // even with the same rounded XYZ. Its UV metadata is not reusable.
            self.vertices.clear();
            let offsets = self
                .offsets
                .map(|o| o.ok_or("Missing raw building offset"))
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;
            let count = if key == 0x030018a4 { 3 } else { 4 };
            let material = crate::building_material::Input::read(m, m.cpu.regs[0], count as usize)?;
            let mut vertices = Vec::new();
            for i in 0..count {
                let src = bytes(m, m.cpu.regs[0] + i * 6, 6)?;
                let raw: [i32; 3] = std::array::from_fn(|a| {
                    i16::from_le_bytes(src[a * 2..a * 2 + 2].try_into().unwrap()) as i32
                });
                let q: [i32; 3] = std::array::from_fn(|a| offsets[a] >> 16);
                vertices.push(Vertex {
                    uv64: material.uv64(i as usize),
                    integer: [
                        raw[0].wrapping_sub(q[0]),
                        raw[1].wrapping_add(q[1]),
                        q[2].wrapping_sub(raw[2]),
                    ],
                    precise: [
                        raw[0] as f64 - offsets[0] as f64 / 65536.0,
                        raw[1] as f64 + offsets[1] as f64 / 65536.0,
                        offsets[2] as f64 / 65536.0 - raw[2] as f64,
                    ],
                });
            }
            self.pending = Some(Transform {
                target: m.cpu.regs[14],
                dest: m.cpu.regs[1],
                vertices,
                material,
            });
        }
        if [0x03001720, 0x030017f8].contains(&key) && word(m, key)? as u32 == 0xe8b20038 {
            if self.projection.is_some() {
                return Err("Nested building projection".into());
            }
            let addr = m.cpu.regs[2];
            let v = [word(m, addr)?, word(m, addr + 4)?, word(m, addr + 8)?];
            if let Some(uv) = self
                .vertices
                .get(&addr)
                .filter(|vertex| vertex.integer == v)
                .and_then(|vertex| vertex.uv64)
            {
                let actual = [word(m, addr + 12)?, word(m, addr + 16)?];
                if actual != uv {
                    return Err(format!(
                        "64x64 UV mismatch at {addr:08x}: {actual:?} != {uv:?}"
                    ));
                }
                self.uv64_checked += 1;
            }
            if v[2] <= 0 {
                return Err("Building projection has nonpositive depth".into());
            }
            let address = m.cpu.regs[8]
                .checked_add(
                    (v[2] as u32)
                        .checked_mul(2)
                        .ok_or("Reciprocal offset overflow")?,
                )
                .ok_or("Reciprocal address overflow")?;
            let reciprocal = u16::from_le_bytes(bytes(m, address, 2)?.try_into().unwrap()) as i32;
            let scale = reciprocal.wrapping_mul(118);
            let expected = [
                (scale.wrapping_mul(v[0]) >> 16).wrapping_add(120),
                (scale.wrapping_neg().wrapping_mul(v[1]) >> 16).wrapping_add(120),
            ];
            let precise = self
                .vertices
                .get(&addr)
                .filter(|vertex| vertex.integer == v)
                .map(|vertex| project(vertex.precise));
            if precise.is_some() {
                self.mapped_projections += 1;
            } else {
                self.unmapped_projections += 1;
            }
            self.projection = Some(Projection {
                target: if key == 0x03001720 {
                    0x03001754
                } else {
                    0x03001828
                },
                expected,
                precise,
            });
        }
        if self.projection.as_ref().is_some_and(|p| p.target == key)
            && word(m, key)? as u32 == 0xe8a90060
        {
            let p = self.projection.take().unwrap();
            let actual = [m.cpu.regs[5] as i32, m.cpu.regs[6] as i32];
            if actual != p.expected {
                return Err(format!(
                    "Building screen projection mismatch: {actual:?} != {:?}",
                    p.expected
                ));
            }
            self.projections_checked += 1;
            let precise = p
                .precise
                .map(|v| format!("{:.9},{:.9}", v[0], v[1]))
                .unwrap_or_else(|| ",".into());
            self.rows.push(format!(
                "{},{},{},{precise}",
                self.projections_checked, actual[0], actual[1]
            ));
        }
        Ok(())
    }
    pub fn export(&self, source: &MemMap, dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if self.pending.is_some()
            || self.projection.is_some()
            || self.vertices_checked == 0
            || self.faces.is_empty()
        {
            return Err(format!("Building observation incomplete: pending={} projection={} vertices={} projections={} faces={}", self.pending.is_some(), self.projection.is_some(), self.vertices_checked, self.projections_checked, self.faces.len()).into());
        }
        let camera = [0x14, 0x18, 0x1c]
            .map(|o| i32::from_le_bytes(source.iwram[o..o + 4].try_into().unwrap()));
        if self.camera != Some(camera) {
            return Err("Building export camera differs from upload camera".into());
        }
        std::fs::create_dir(dir)?;
        let objects = self.objects.capture(source, &dir.join("objects"))?;
        self.materials
            .export(source, &dir.join("materials"), &objects)?;
        std::fs::write(
            dir.join("uv64-checks.txt"),
            format!("mapped_vertex_uv_checks={}\n", self.uv64_checked),
        )?;
        std::fs::write(dir.join("checks.txt"),format!("vertices_checked={}\nprojections_checked={}\nmapped_projections={}\nunmapped_projections={}\nselected_faces={}\nraw_camera={camera:?}\nRead-only; original face selection; outlines only.\n",self.vertices_checked,self.projections_checked,self.mapped_projections,self.unmapped_projections,self.faces.len()))?;
        let mut rows = std::fs::File::create_new(dir.join("projections.csv"))?;
        writeln!(
            rows,
            "vertex,original_x,original_y,continuous_x,continuous_y"
        )?;
        for row in &self.rows {
            writeln!(rows, "{row}")?;
        }
        let mut motions = std::fs::File::create_new(dir.join("camera.csv"))?;
        writeln!(
            motions,
            "delta_x_raw,visible_faces,projected_vertices,changed_pixels_3x"
        )?;
        let mut original = None;
        for delta in [0i32, 16384, 32768, 49152, 65536] {
            let mut image = crate::ground_projection::continuous_image(source, 3, delta, 0);
            let mut svg=String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 240 160\" width=\"720\" height=\"480\"><rect width=\"240\" height=\"160\" fill=\"#111923\"/><g fill=\"none\" stroke=\"#52ffff\" stroke-width=\"0.25\">");
            let mut vertices =
                std::fs::File::create_new(dir.join(format!("vertices-{delta}.csv")))?;
            writeln!(
                vertices,
                "face,vertex,camera_x,camera_y,depth,screen_x,screen_y"
            )?;
            let mut faces = 0;
            let mut points = 0;
            for (index, face) in self.faces.iter().enumerate() {
                let mut poly = face.clone();
                for v in &mut poly {
                    v[0] -= delta as f64 / 65536.0;
                }
                let poly = clip(poly);
                if poly.len() < 3 {
                    continue;
                }
                faces += 1;
                let mut projected = Vec::new();
                svg.push_str("<polygon points=\"");
                for (i, v) in poly.iter().enumerate() {
                    let p = project(*v);
                    projected.push(p);
                    points += 1;
                    use std::fmt::Write as _;
                    write!(svg, "{:.9},{:.9} ", p[0], p[1])?;
                    writeln!(
                        vertices,
                        "{index},{i},{},{},{},{},{}",
                        v[0], v[1], v[2], p[0], p[1]
                    )?;
                }
                svg.push_str("\"/>");
                for i in 0..projected.len() {
                    line(
                        &mut image,
                        720,
                        480,
                        projected[i],
                        projected[(i + 1) % projected.len()],
                    );
                }
            }
            svg.push_str("</g></svg>");
            std::fs::write(dir.join(format!("outline-{delta}.svg")), svg)?;
            crate::scene_replay::ppm(
                &dir.join(format!("ground-outline-{delta}.ppm")),
                &image,
                720,
                480,
            )?;
            let changes = original
                .as_ref()
                .map(|base: &Vec<u16>| base.iter().zip(&image).filter(|(a, b)| a != b).count())
                .unwrap_or(0);
            writeln!(motions, "{delta},{faces},{points},{changes}")?;
            if delta == 0 {
                original = Some(image);
            }
        }
        Ok(())
    }
}
fn distance(v: [f64; 3], plane: usize) -> f64 {
    match plane {
        0 => v[2] - 32.0,
        1 => v[2] + v[0],
        2 => v[2] - v[0],
        3 => v[2] + v[1],
        _ => v[2] - v[1],
    }
}
fn clip(mut vertices: Vec<[f64; 3]>) -> Vec<[f64; 3]> {
    for plane in 0..5 {
        let Some(mut a) = vertices.last().copied() else {
            break;
        };
        let mut next = Vec::new();
        for b in vertices {
            let da = distance(a, plane);
            let db = distance(b, plane);
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                next.push(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t));
            }
            if db >= 0.0 {
                next.push(b);
            }
            a = b;
        }
        vertices = next;
    }
    vertices
}
fn line(image: &mut [u16], width: usize, height: usize, a: [f64; 2], b: [f64; 2]) {
    let (ax, ay, bx, by) = (a[0] * 3.0, a[1] * 3.0, b[0] * 3.0, b[1] * 3.0);
    let steps = (ax - bx).abs().max((ay - by).abs()).ceil() as usize;
    for i in 0..=steps {
        let t = if steps == 0 {
            0.0
        } else {
            i as f64 / steps as f64
        };
        let x = (ax + (bx - ax) * t).round() as i32;
        let y = (ay + (by - ay) * t).round() as i32;
        if x >= 0 && y >= 0 && x < width as i32 && y < height as i32 {
            image[y as usize * width + x as usize] = 0x7fe0;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wide_model_bookkeeping_admits_side_only_boxes_and_keeps_central_boxes() {
        for (left, right, expected) in [
            (-30, -1, [0, 8]),
            (241, 270, [232, 240]),
            (0, 239, [0, 239]),
            (-10, 20, [-10, 20]),
            (220, 250, [220, 250]),
            (-80, -40, [-80, -40]),
            (280, 300, [280, 300]),
        ] {
            let mut registers = [0u32; 16];
            registers[5] = left as u32;
            registers[4] = right as u32;
            super::wide_model_edge_tile(&mut registers);
            assert_eq!([registers[5] as i32, registers[4] as i32], expected);
        }
    }

    use super::*;
    #[test]
    fn clipped_triangle_does_not_reuse_preceding_quads_fourth_vertex_uv() {
        let mut m = Machine::new(Vec::new());
        m.bus.iwram[0x18a8..0x18ac].copy_from_slice(&0xe8b20038u32.to_le_bytes());
        m.bus.iwram[0x1720..0x1724].copy_from_slice(&0xe8b20038u32.to_le_bytes());
        let stale = Vertex {
            integer: [99, -49, 119],
            precise: [99.0, -49.0, 119.0],
            uv64: Some([63, 63]),
        };
        let mut observer = Observer::default();
        observer.offsets = [Some(0), Some(0), Some(200 * 65536)];
        observer.vertices.insert(0x03007c40, stale);
        m.cpu.regs[0] = 0x02000004;
        m.cpu.regs[1] = 0x03007c04;
        m.cpu.regs[14] = 0x0800abcd;
        observer.observe(&m, 0x030018a4).unwrap();
        for i in 0..3 {
            m.bus.iwram[0x7c04 + i * 20 + 8..0x7c04 + i * 20 + 12]
                .copy_from_slice(&200i32.to_le_bytes());
        }
        m.cpu.regs[0] = 1; // culled source face need not enter the material batch
        observer.observe(&m, 0x0800abcd).unwrap();
        assert_eq!(observer.vertices.len(), 3);
        assert!(!observer.vertices.contains_key(&0x03007c40));
        for (i, v) in [99i32, -49, 119, 109, 0].into_iter().enumerate() {
            m.bus.iwram[0x7c40 + i * 4..0x7c44 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        m.cpu.regs[2] = 0x03007c40;
        m.cpu.regs[8] = 0x02001000;
        observer.observe(&m, 0x03001720).unwrap();
        assert_eq!(observer.unmapped_projections, 1);
        assert_eq!(observer.uv64_checked, 0);
        let mut current = Observer::default();
        current.vertices.insert(0x03007c40, stale);
        assert!(current
            .observe(&m, 0x03001720)
            .unwrap_err()
            .contains("64x64 UV mismatch"));
    }
    #[test]
    fn hook_filter_includes_dynamic_transform_and_projection_returns() {
        let mut observer = Observer::default();
        assert!(!observer.interested(0x08000011));
        observer.pending = Some(Transform {
            target: 0x08000011,
            dest: 0,
            vertices: Vec::new(),
            material: crate::building_material::Input::read(
                &Machine::new(vec![0; 64]),
                0x08000004,
                3,
            )
            .unwrap(),
        });
        observer.projection = Some(Projection {
            target: 0x08000021,
            expected: [0; 2],
            precise: None,
        });
        assert!(observer.interested(0x08000011));
        assert!(observer.interested(0x08000021));
        assert!(!observer.interested(0x08000031));
    }
    #[test]
    fn camera_motion_retains_fraction_and_changes_with_depth() {
        let a = project([20.0, 0.0, 100.0]);
        let b = project([19.75, 0.0, 100.0]);
        assert!((a[0] - b[0] - 0.295).abs() < 1e-10);
        let far_a = project([20.0, 0.0, 200.0]);
        let far_b = project([19.75, 0.0, 200.0]);
        assert!((a[0] - b[0] - 2.0 * (far_a[0] - far_b[0])).abs() < 1e-10);
    }
    #[test]
    fn clipping_retains_fractional_intersections_and_supported_depth() {
        let polygon = clip(vec![
            [-10.0, -5.0, 20.0],
            [20.0, -5.0, 60.0],
            [0.0, 20.0, 60.0],
        ]);
        assert!(polygon.len() >= 3);
        for v in polygon {
            for plane in 0..5 {
                assert!(distance(v, plane) >= -1e-9);
            }
            assert!(project(v).iter().all(|x| x.is_finite()));
        }
    }
}
