//! CPU-side mesh data uploaded once by the renderer.

use bytemuck::{Pod, Zeroable};

/// Vertex layout shared by every built-in mesh.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    /// Object-space position.
    pub position: [f32; 3],
    /// Object-space unit normal.
    pub normal: [f32; 3],
    /// Texture coordinate, origin at the top-left.
    pub uv: [f32; 2],
    /// Eight 16-bit joint palette indices (glTF JOINTS_0 and JOINTS_1).
    pub joints: [u16; 8],
    /// Eight normalized linear-blend skinning weights (glTF WEIGHTS_0 and WEIGHTS_1).
    pub weights: [f32; 8],
}

/// Vertices and 32-bit triangle indices (counter-clockwise front faces).
pub struct MeshData {
    /// Vertex list.
    pub vertices: Vec<Vertex>,
    /// Triangle indices into `vertices`.
    pub indices: Vec<u32>,
}

/// Unit cube centered on the origin with per-face normals.
pub fn cube() -> MeshData {
    // (normal, u, v) with u x v == normal keeps every face counter-clockwise from outside.
    const FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    const CORNERS: [(f32, f32); 4] = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, u, v) in FACES {
        let base = vertices.len() as u32;
        for (cu, cv) in CORNERS {
            vertices.push(Vertex {
                position: [
                    normal[0] * 0.5 + u[0] * cu + v[0] * cv,
                    normal[1] * 0.5 + u[1] * cu + v[1] * cv,
                    normal[2] * 0.5 + u[2] * cu + v[2] * cv,
                ],
                normal,
                uv: [cu + 0.5, 0.5 - cv],
                joints: [0; 8],
                weights: [0.0; 8],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    MeshData { vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::cube;
    use glam::Vec3;

    #[test]
    fn cube_has_outward_counter_clockwise_faces() {
        let mesh = cube();
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
        for triangle in mesh.indices.chunks(3) {
            let p: Vec<Vec3> = triangle
                .iter()
                .map(|&i| Vec3::from(mesh.vertices[i as usize].position))
                .collect();
            let declared = Vec3::from(mesh.vertices[triangle[0] as usize].normal);
            let geometric = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            assert!(
                (geometric - declared).length() < 1e-6,
                "{geometric} vs {declared}"
            );
            assert!(
                p[0].dot(declared) > 0.0,
                "face must point away from the center"
            );
        }
    }
}
