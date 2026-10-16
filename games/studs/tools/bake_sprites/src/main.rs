use image::{ImageBuffer, Rgba, RgbaImage};
use std::{env, error::Error, fs, fs::File, io::Write, path::Path};

const VIEWS: usize = 16;
const FRAMES: usize = 8;
const STATES: usize = 5;
const CELL_W: usize = 48;
const CELL_H: usize = 96;

#[derive(Clone, Copy)]
struct V3(f32, f32, f32);

impl V3 {
    fn add(self, b: Self) -> Self {
        Self(self.0 + b.0, self.1 + b.1, self.2 + b.2)
    }
    fn sub(self, b: Self) -> Self {
        Self(self.0 - b.0, self.1 - b.1, self.2 - b.2)
    }
    fn scale(self, s: f32) -> Self {
        Self(self.0 * s, self.1 * s, self.2 * s)
    }
    fn dot(self, b: Self) -> f32 {
        self.0 * b.0 + self.1 * b.1 + self.2 * b.2
    }
    fn cross(self, b: Self) -> Self {
        Self(
            self.1 * b.2 - self.2 * b.1,
            self.2 * b.0 - self.0 * b.2,
            self.0 * b.1 - self.1 * b.0,
        )
    }
    fn normalized(self) -> Self {
        self.scale(1.0 / self.dot(self).sqrt().max(1e-8))
    }
}

#[derive(Clone, Copy)]
struct Vertex {
    p: V3,
    uv: [f32; 2],
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "inspect".to_owned());
    let path = args.next().ok_or("usage: bake_sprites inspect <file.fbx> | export-obj <file.fbx> <out.obj> | bake <file.fbx> <output-dir> [albedo.png]")?;
    let scene = ufbx::load_file(&path, ufbx::LoadOpts::default())
        .map_err(|error| format!("failed to parse {path}: {}", error.description))?;
    println!("scene: {}", path);
    println!(
        "nodes: {}, meshes: {}, skins: {}, anim stacks: {}",
        scene.nodes.len(),
        scene.meshes.len(),
        scene.skin_deformers.len(),
        scene.anim_stacks.len()
    );
    for node in &scene.nodes {
        println!("node: {}", node.element.name);
    }
    for mesh in &scene.meshes {
        println!(
            "mesh: {} vertices={} faces={} materials={}",
            mesh.element.name, mesh.vertices.count, mesh.faces.count, mesh.materials.count
        );
        if let Some(node) = scene.nodes.iter().find(|node| {
            node.mesh
                .as_ref()
                .is_some_and(|m| m.element.typed_id == mesh.element.typed_id)
        }) {
            println!(
                "  node: {}; geo-world={:?}",
                node.element.name, node.geometry_to_world
            );
        }
    }
    for skin in &scene.skin_deformers {
        println!(
            "skin: {} clusters={}",
            skin.element.name, skin.clusters.count
        );
        for cluster in &skin.clusters {
            let bone = cluster
                .bone_node
                .as_ref()
                .map(|n| n.element.name.as_ref())
                .unwrap_or("<unbound>");
            println!(
                "  bone: {bone}; weighted vertices={}",
                cluster.vertices.count
            );
        }
    }
    for stack in &scene.anim_stacks {
        println!("animation: {}", stack.element.name);
    }
    if command == "export-obj" {
        let output = args
            .next()
            .ok_or("export-obj requires an output OBJ path")?;
        let mut obj = File::create(output)?;
        let mut offset = 1usize;
        for mesh in &scene.meshes {
            writeln!(obj, "o {}", mesh.element.name)?;
            let matrix = scene
                .nodes
                .iter()
                .find(|node| {
                    node.mesh
                        .as_ref()
                        .is_some_and(|m| m.element.typed_id == mesh.element.typed_id)
                })
                .map(|node| node.geometry_to_world)
                .unwrap_or_default();
            for i in 0..mesh.num_vertices {
                let p = mesh.vertices[i];
                let x = matrix.m00 * p.x + matrix.m01 * p.y + matrix.m02 * p.z + matrix.m03;
                let y = matrix.m10 * p.x + matrix.m11 * p.y + matrix.m12 * p.z + matrix.m13;
                let z = matrix.m20 * p.x + matrix.m21 * p.y + matrix.m22 * p.z + matrix.m23;
                writeln!(obj, "v {} {} {}", x, y, z)?;
            }
            for face in &mesh.faces {
                if face.num_indices < 3 {
                    continue;
                }
                let begin = face.index_begin as usize;
                let first = mesh.vertex_indices[begin] as usize + offset;
                for j in 1..face.num_indices as usize - 1 {
                    let b = mesh.vertex_indices[begin + j] as usize + offset;
                    let c = mesh.vertex_indices[begin + j + 1] as usize + offset;
                    writeln!(obj, "f {first} {b} {c}")?;
                }
            }
            offset += mesh.num_vertices;
        }
    } else if command == "bake" {
        let output = args.next().ok_or("bake requires an output directory")?;
        let texture = args
            .next()
            .map(image::open)
            .transpose()?
            .map(|image| image.to_rgba8());
        bake_scene(&scene, Path::new(&output), texture.as_ref())?;
    } else if command != "inspect" {
        return Err(format!("unknown command {command}").into());
    }
    Ok(())
}

fn matrix_point(m: ufbx::Matrix, p: V3) -> V3 {
    V3(
        (m.m00 * p.0 as f64 + m.m01 * p.1 as f64 + m.m02 * p.2 as f64 + m.m03) as f32,
        (m.m10 * p.0 as f64 + m.m11 * p.1 as f64 + m.m12 * p.2 as f64 + m.m13) as f32,
        (m.m20 * p.0 as f64 + m.m21 * p.1 as f64 + m.m22 * p.2 as f64 + m.m23) as f32,
    )
}

// FBX sources use Y-up; STUDS scenes use Z-up.
fn to_game(p: V3) -> V3 {
    V3(p.0, p.2, p.1)
}

fn rotate_about(p: V3, pivot: V3, axis: usize, angle: f32) -> V3 {
    let q = p.sub(pivot);
    let (s, c) = angle.sin_cos();
    let r = match axis {
        0 => V3(q.0, c * q.1 - s * q.2, s * q.1 + c * q.2),
        1 => V3(c * q.0 + s * q.2, q.1, -s * q.0 + c * q.2),
        _ => V3(c * q.0 - s * q.1, s * q.0 + c * q.1, q.2),
    };
    pivot.add(r)
}

fn joint_angle(name: &str, state: usize, phase: f32) -> (usize, f32) {
    let side = if name.to_ascii_lowercase().contains("left") {
        1.0
    } else {
        -1.0
    };
    let cycle = phase * std::f32::consts::TAU;
    if name.contains("leg") || name.contains("knee") || name.contains("ankle") {
        let amount = match state {
            0 => 0.025,
            1 => 0.34,
            2 => 0.68,
            3 => -0.45,
            _ => 0.10,
        };
        (0, side * amount * cycle.sin())
    } else if name.contains("arm")
        || name.contains("elbow")
        || name.contains("wrist")
        || name.contains("shoulder")
    {
        let amount = match state {
            0 => 0.035,
            1 => 0.46,
            2 => 0.82,
            3 => 0.95,
            _ => 0.18,
        };
        let sign = if state == 3 { -side } else { side };
        (0, sign * amount * cycle.sin())
    } else if name.contains("Spine") || name.contains("Chest") || name.contains("Hips") {
        let amount = match state {
            0 => 0.025,
            1 => 0.04,
            2 => 0.12,
            3 => 0.34,
            _ => 0.12,
        };
        (1, amount * (cycle.sin() + 0.35))
    } else if name.contains("Neck") || name.contains("Head") {
        let amount = if state == 4 {
            0.22
        } else if state == 3 {
            -0.20
        } else {
            0.035
        };
        (1, amount * cycle.sin())
    } else {
        let amount = if state == 4 { 0.24 } else { 0.08 };
        (1, amount * cycle.sin())
    }
}

fn skinned_vertices(
    scene: &ufbx::Scene,
    mesh: &ufbx::Mesh,
    state: usize,
    phase: f32,
) -> Vec<Vertex> {
    let mesh_node = scene.nodes.iter().find(|node| {
        node.mesh
            .as_ref()
            .is_some_and(|m| m.element.typed_id == mesh.element.typed_id)
    });
    let mesh_matrix = mesh_node
        .map(|node| node.geometry_to_world)
        .unwrap_or_default();
    let Some(skin) = mesh.skin_deformers.iter().next() else {
        return (0..mesh.num_vertices)
            .map(|i| {
                let p = to_game(matrix_point(
                    mesh_matrix,
                    V3(
                        mesh.vertices[i].x as f32,
                        mesh.vertices[i].y as f32,
                        mesh.vertices[i].z as f32,
                    ),
                ));
                let uv = if mesh.vertex_uv.exists {
                    let uv = mesh.vertex_uv[i];
                    [uv.x as f32, uv.y as f32]
                } else {
                    [0.5, 0.5]
                };
                Vertex { p, uv }
            })
            .collect();
    };
    (0..mesh.num_vertices)
        .map(|i| {
            let source = V3(
                mesh.vertices[i].x as f32,
                mesh.vertices[i].y as f32,
                mesh.vertices[i].z as f32,
            );
            let base = matrix_point(mesh_matrix, source);
            let mut result = V3(0.0, 0.0, 0.0);
            let mut total = 0.0;
            if i < skin.vertices.count {
                let sv = skin.vertices[i];
                for wi in sv.weight_begin as usize..(sv.weight_begin + sv.num_weights) as usize {
                    let weight = skin.weights[wi];
                    if weight.weight <= 0.0 {
                        continue;
                    }
                    let Some(cluster) = skin.clusters.get(weight.cluster_index as usize) else {
                        continue;
                    };
                    let Some(bone) = cluster.bone_node.as_ref() else {
                        continue;
                    };
                    let bind_local = matrix_point(cluster.geometry_to_bone, source);
                    let bone_world = matrix_point(bone.node_to_world, bind_local);
                    let pivot = to_game(V3(
                        bone.node_to_world.m03 as f32,
                        bone.node_to_world.m13 as f32,
                        bone.node_to_world.m23 as f32,
                    ));
                    let mut posed = to_game(bone_world);
                    let (axis, angle) = joint_angle(bone.element.name.as_ref(), state, phase);
                    posed = rotate_about(posed, pivot, axis, angle);
                    result = result.add(posed.scale(weight.weight as f32));
                    total += weight.weight as f32;
                }
            }
            if total < 1.0 {
                result = result.add(to_game(base).scale(1.0 - total));
            }
            let uv = if mesh.vertex_uv.exists {
                let uv = mesh.vertex_uv[i];
                [uv.x as f32, uv.y as f32]
            } else {
                [0.5, 0.5]
            };
            Vertex { p: result, uv }
        })
        .collect()
}

fn sample_texture(texture: Option<&RgbaImage>, uv: [f32; 2]) -> [u8; 3] {
    if let Some(texture) = texture {
        let x = (uv[0].rem_euclid(1.0) * texture.width() as f32) as u32 % texture.width().max(1);
        let y = ((1.0 - uv[1].rem_euclid(1.0)) * texture.height() as f32) as u32
            % texture.height().max(1);
        let p = texture.get_pixel(x, y).0;
        [p[0], p[1], p[2]]
    } else {
        [105, 91, 86]
    }
}

#[allow(clippy::too_many_arguments)] // Raster state is deliberately explicit and allocation-free.
fn draw_triangle(
    albedo: &mut RgbaImage,
    normal: &mut RgbaImage,
    depth: &mut [f32],
    origin: (u32, u32),
    cell: (u32, u32),
    pts: [Vertex; 3],
    camera_angle: f32,
    scale: f32,
    min_z: f32,
    texture: Option<&RgbaImage>,
) {
    let (sn, cs) = camera_angle.sin_cos();
    let project = |p: V3| {
        let sx = p.0 * cs + p.1 * sn;
        let dep = -p.0 * sn + p.1 * cs;
        (
            cell.0 as f32 * 0.5 + sx * scale,
            cell.1 as f32 - (p.2 - min_z) * scale - 2.0,
            dep,
        )
    };
    let projected = pts.map(|v| project(v.p));
    let (x0, y0, _) = projected[0];
    let (x1, y1, _) = projected[1];
    let (x2, y2, _) = projected[2];
    let area = (x1 - x0) * (y2 - y0) - (y1 - y0) * (x2 - x0);
    if area.abs() < 1e-6 {
        return;
    }
    let min_x = x0.min(x1).min(x2).floor().max(0.0) as u32;
    let max_x = x0.max(x1).max(x2).ceil().min(cell.0 as f32 - 1.0) as u32;
    let min_y = y0.min(y1).min(y2).floor().max(0.0) as u32;
    let max_y = y0.max(y1).max(y2).ceil().min(cell.1 as f32 - 1.0) as u32;
    let e1 = pts[1].p.sub(pts[0].p);
    let e2 = pts[2].p.sub(pts[0].p);
    let face_n = e1.cross(e2).normalized();
    let light = V3(sn, -cs, 0.35).normalized();
    let shade = (0.28 + 0.72 * face_n.dot(light).abs()).clamp(0.0, 1.0);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let w0 = ((x1 - px) * (y2 - py) - (y1 - py) * (x2 - px)) / area;
            let w1 = ((x2 - px) * (y0 - py) - (y2 - py) * (x0 - px)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 {
                continue;
            }
            let dep = w0 * projected[0].2 + w1 * projected[1].2 + w2 * projected[2].2;
            let index = (y + origin.1) as usize * albedo.width() as usize + (x + origin.0) as usize;
            if dep <= depth[index] {
                continue;
            }
            depth[index] = dep;
            let uv = [
                w0 * pts[0].uv[0] + w1 * pts[1].uv[0] + w2 * pts[2].uv[0],
                w0 * pts[0].uv[1] + w1 * pts[1].uv[1] + w2 * pts[2].uv[1],
            ];
            let tex = sample_texture(texture, uv);
            let noise = ((x.wrapping_mul(13) + y.wrapping_mul(7)) % 11) as i32 - 5;
            let color = Rgba([
                ((tex[0] as f32 * 0.48 + 42.0) * shade) as u8,
                ((tex[1] as f32 * 0.43 + 35.0) * shade) as u8,
                ((tex[2] as f32 * 0.40 + 38.0) * shade) as u8,
                255,
            ]);
            let _ = noise;
            albedo.put_pixel(x + origin.0, y + origin.1, color);
            normal.put_pixel(
                x + origin.0,
                y + origin.1,
                Rgba([
                    ((face_n.0 * 0.5 + 0.5) * 255.0) as u8,
                    ((face_n.1 * 0.5 + 0.5) * 255.0) as u8,
                    ((face_n.2 * 0.5 + 0.5) * 255.0) as u8,
                    255,
                ]),
            );
        }
    }
}

fn bake_scene(
    scene: &ufbx::Scene,
    out: &Path,
    texture: Option<&RgbaImage>,
) -> Result<(), Box<dyn Error>> {
    let mesh = scene.meshes.first().ok_or("FBX contains no mesh")?;
    fs::create_dir_all(out)?;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    let mut radius: f32 = 0.0;
    for state in 0..STATES {
        for frame in 0..FRAMES {
            let vertices = skinned_vertices(scene, mesh, state, frame as f32 / FRAMES as f32);
            for v in vertices {
                min_z = min_z.min(v.p.2);
                max_z = max_z.max(v.p.2);
                radius = radius.max((v.p.0 * v.p.0 + v.p.1 * v.p.1).sqrt());
            }
        }
    }
    let width = (VIEWS * CELL_W) as u32;
    let height = (STATES * FRAMES * CELL_H) as u32;
    let mut albedo = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 0]));
    let mut normal = ImageBuffer::from_pixel(width, height, Rgba([128, 128, 255, 0]));
    let mut emissive: RgbaImage = ImageBuffer::from_pixel(width, height, Rgba([0u8, 0, 0, 0]));
    let scale = ((CELL_W as f32 * 0.82) / (radius.max(0.01) * 2.0))
        .min((CELL_H as f32 * 0.94) / (max_z - min_z).max(0.01));
    let mut depth = vec![f32::NEG_INFINITY; width as usize * height as usize];
    for state in 0..STATES {
        for frame in 0..FRAMES {
            let vertices = skinned_vertices(scene, mesh, state, frame as f32 / FRAMES as f32);
            for view in 0..VIEWS {
                let angle = view as f32 * std::f32::consts::TAU / VIEWS as f32;
                let cell_x = (view * CELL_W) as u32;
                let cell_y = ((state * FRAMES + frame) * CELL_H) as u32;
                for face in &mesh.faces {
                    if face.num_indices < 3 {
                        continue;
                    }
                    let start = face.index_begin as usize;
                    let mut ids = Vec::new();
                    for j in 0..face.num_indices as usize {
                        ids.push(mesh.vertex_indices[start + j] as usize);
                    }
                    for j in 1..ids.len() - 1 {
                        if ids[0] >= vertices.len()
                            || ids[j] >= vertices.len()
                            || ids[j + 1] >= vertices.len()
                        {
                            continue;
                        }
                        draw_triangle(
                            &mut albedo,
                            &mut normal,
                            &mut depth,
                            (cell_x, cell_y),
                            (CELL_W as u32, CELL_H as u32),
                            [vertices[ids[0]], vertices[ids[j]], vertices[ids[j + 1]]],
                            angle,
                            scale,
                            min_z,
                            texture,
                        );
                    }
                }
                // The centered head bone gets two emissive pinpoints in the frontal half of the view set.
                if view < VIEWS / 2 {
                    if let Some(head) = scene.nodes.iter().find(|n| {
                        n.element
                            .name
                            .as_ref()
                            .to_ascii_lowercase()
                            .contains("head")
                    }) {
                        let p = to_game(V3(
                            head.node_to_world.m03 as f32,
                            head.node_to_world.m13 as f32,
                            head.node_to_world.m23 as f32,
                        ));
                        let (sn, cs) = angle.sin_cos();
                        let sx = CELL_W as f32 * 0.5 + (p.0 * cs + p.1 * sn) * scale;
                        let sy = CELL_H as f32 - (p.2 - min_z) * scale - 2.0;
                        for eye in [-2.5f32, 2.5] {
                            for dy in -1..=1 {
                                for dx in -1..=1 {
                                    let x = (cell_x as f32 + sx + eye + dx as f32) as i32;
                                    let y = (cell_y as f32 + sy + dy as f32) as i32;
                                    if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height
                                    {
                                        emissive.put_pixel(
                                            x as u32,
                                            y as u32,
                                            Rgba([255, 255, 255, 255]),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    albedo.save(out.join("albedo.png"))?;
    normal.save(out.join("normal.png"))?;
    emissive.save(out.join("emissive.png"))?;
    fs::write(out.join("metadata.txt"),format!("views={VIEWS}\nstates=idle,walk,run_lunge,attack,frozen\nframes_per_state={FRAMES}\ncell_width={CELL_W}\ncell_height={CELL_H}\nwidth={width}\nheight={height}\ncoordinate_system=studs-z-up\n"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pose_selector_changes_limbs_for_run_and_frozen_states() {
        assert!(
            joint_angle("Left arm", 2, 0.25).1.abs() > joint_angle("Left arm", 0, 0.25).1.abs()
        );
        assert_ne!(
            joint_angle("Head", 4, 0.25).1,
            joint_angle("Head", 0, 0.25).1
        );
    }

    #[test]
    fn synthetic_triangle_rasterizes_with_depth_and_alpha() {
        let mut albedo = RgbaImage::new(8, 8);
        let mut normal = RgbaImage::new(8, 8);
        let mut depth = vec![f32::NEG_INFINITY; 64];
        let v = |x, y, z| Vertex {
            p: V3(x, y, z),
            uv: [0.5, 0.5],
        };
        let triangle = [v(-1.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 0.0, 2.0)];
        draw_triangle(
            &mut albedo,
            &mut normal,
            &mut depth,
            (0, 0),
            (8, 8),
            triangle,
            0.0,
            2.0,
            0.0,
            None,
        );
        assert!(
            albedo.pixels().any(|p| p[3] != 0),
            "triangle should cover pixels"
        );
        assert!(
            depth.iter().any(|d| d.is_finite()),
            "triangle should write depth"
        );
    }
}
