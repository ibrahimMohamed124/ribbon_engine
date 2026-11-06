//! CPU-side mesh data uploaded once by the renderer.

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

/// Legacy eight-influence CPU input retained for source compatibility.
///
/// At upload time, zero-weight inputs use [`StaticVertex`], while weighted or malformed inputs
/// preserve this full influence data in [`SkinnedVertex`]. Both adapters generate a stable
/// tangent frame and use UV0 as the UV1 fallback. New importers should provide an explicit format.
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

/// Compact non-skinned GPU vertex format with tangent-space and two-UV support.
///
/// Position + normal + UV0 + tangent + UV1 occupy 56 bytes. This remains smaller than
/// [`SkinnedVertex`] and never reserves joint indices or skin weights.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct StaticVertex {
    /// Object-space position.
    pub position: [f32; 3],
    /// Object-space unit normal.
    pub normal: [f32; 3],
    /// First texture coordinate set, origin at the top-left.
    pub uv: [f32; 2],
    /// Object-space tangent xyz and handedness sign in w.
    pub tangent: [f32; 4],
    /// Second texture coordinate set. Falls back to UV0 when absent in glTF.
    pub uv1: [f32; 2],
}

/// GPU vertex format for skinned geometry with eight influences, tangent space and UV1.
///
/// The legacy [`Vertex`] remains an accepted CPU input; this explicit type is what the
/// renderer actually uploads for skinned geometry.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SkinnedVertex {
    /// Object-space position.
    pub position: [f32; 3],
    /// Object-space unit normal.
    pub normal: [f32; 3],
    /// First texture coordinate set.
    pub uv: [f32; 2],
    /// Object-space tangent xyz and handedness sign in w.
    pub tangent: [f32; 4],
    /// Second texture coordinate set.
    pub uv1: [f32; 2],
    /// Eight 16-bit joint palette indices (JOINTS_0 and JOINTS_1).
    pub joints: [u16; 8],
    /// Eight normalized linear-blend skinning weights (WEIGHTS_0 and WEIGHTS_1).
    pub weights: [f32; 8],
}

/// Return whether legacy vertices contain skinning data that must be preserved in the GPU layout.
///
/// A zero-weight legacy mesh is static and can use [`StaticVertex`]. Any non-zero weight (including
/// negative or non-finite values) keeps the compatibility path on [`SkinnedVertex`] so this adapter
/// never silently discards caller-provided skinning data.
pub(crate) fn legacy_vertices_require_skinning(vertices: &[Vertex]) -> bool {
    vertices
        .iter()
        .any(|vertex| vertex.weights.iter().any(|weight| *weight != 0.0))
}

/// Generate a robust tangent frame per vertex from indexed triangles and UV0.
///
/// Degenerate triangles and invalid UV derivatives are skipped. Vertices without a usable
/// accumulated tangent receive a deterministic orthonormal fallback. Mirrored UV handedness is
/// retained in tangent.w; glTF vertex seams must remain split at import time as usual.
pub fn generate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let count = positions.len();
    let mut tangent_sum = vec![Vec3::ZERO; count];
    let mut bitangent_sum = vec![Vec3::ZERO; count];

    for triangle in indices.as_chunks::<3>().0 {
        let [i0, i1, i2] = triangle.map(|index| index as usize);
        if i0 >= count
            || i1 >= count
            || i2 >= count
            || i0 >= normals.len()
            || i1 >= normals.len()
            || i2 >= normals.len()
            || i0 >= uvs.len()
            || i1 >= uvs.len()
            || i2 >= uvs.len()
        {
            continue;
        }
        let p0 = Vec3::from_array(positions[i0]);
        let p1 = Vec3::from_array(positions[i1]);
        let p2 = Vec3::from_array(positions[i2]);
        let uv0 = Vec2::from_array(uvs[i0]);
        let uv1 = Vec2::from_array(uvs[i1]);
        let uv2 = Vec2::from_array(uvs[i2]);
        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let delta1 = uv1 - uv0;
        let delta2 = uv2 - uv0;
        let determinant = delta1.x * delta2.y - delta1.y * delta2.x;
        if !determinant.is_finite() || determinant.abs() <= 1.0e-8 {
            continue;
        }
        let inverse = determinant.recip();
        let tangent = (edge1 * delta2.y - edge2 * delta1.y) * inverse;
        let bitangent = (edge2 * delta1.x - edge1 * delta2.x) * inverse;
        if !tangent.is_finite() || !bitangent.is_finite() {
            continue;
        }
        // Area weighting avoids tiny triangles dominating a vertex shared with larger faces.
        let area = edge1.cross(edge2).length().max(1.0e-8);
        for index in [i0, i1, i2] {
            tangent_sum[index] += tangent * area;
            bitangent_sum[index] += bitangent * area;
        }
    }

    (0..count)
        .map(|index| {
            let normal = normals
                .get(index)
                .copied()
                .map(Vec3::from_array)
                .filter(|normal| normal.is_finite() && normal.length_squared() > 1.0e-12)
                .map(Vec3::normalize)
                .unwrap_or(Vec3::Z);
            let projected = tangent_sum[index] - normal * normal.dot(tangent_sum[index]);
            let tangent = if projected.is_finite() && projected.length_squared() > 1.0e-12 {
                projected.normalize()
            } else {
                fallback_tangent(normal)
            };
            let handedness = if bitangent_sum[index].is_finite()
                && normal.cross(tangent).dot(bitangent_sum[index]) < 0.0
            {
                -1.0
            } else {
                1.0
            };
            [tangent.x, tangent.y, tangent.z, handedness]
        })
        .collect()
}

fn fallback_tangent(normal: Vec3) -> Vec3 {
    let axis = if normal.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let tangent = axis.cross(normal);
    if tangent.length_squared() > 1.0e-12 {
        tangent.normalize()
    } else {
        Vec3::Z.cross(normal).normalize_or_zero()
    }
}

/// Vertices and 32-bit triangle indices (counter-clockwise front faces).
pub struct MeshData {
    /// Legacy CPU vertex list.
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
    use super::{cube, generate_tangents, legacy_vertices_require_skinning};
    use glam::Vec3;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn compact_static_vertex_contains_tangent_and_two_uv_sets() {
        use super::StaticVertex;
        assert_eq!(size_of::<StaticVertex>(), 56);
        assert_eq!(align_of::<StaticVertex>(), 4);
        assert_eq!(offset_of!(StaticVertex, position), 0);
        assert_eq!(offset_of!(StaticVertex, normal), 12);
        assert_eq!(offset_of!(StaticVertex, uv), 24);
        assert_eq!(offset_of!(StaticVertex, tangent), 32);
        assert_eq!(offset_of!(StaticVertex, uv1), 48);
    }

    #[test]
    fn skinned_vertex_keeps_eight_influences_after_tangent_and_uv1() {
        use super::SkinnedVertex;
        assert_eq!(size_of::<SkinnedVertex>(), 104);
        assert_eq!(offset_of!(SkinnedVertex, tangent), 32);
        assert_eq!(offset_of!(SkinnedVertex, uv1), 48);
        assert_eq!(offset_of!(SkinnedVertex, joints), 56);
        assert_eq!(offset_of!(SkinnedVertex, weights), 72);
    }

    #[test]
    fn tangent_generation_is_finite_and_preserves_mirrored_uv_handedness() {
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let normals = [[0.0, 0.0, 1.0]; 3];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let tangents = generate_tangents(&positions, &normals, &uvs, &[0, 1, 2]);
        assert_eq!(tangents.len(), 3);
        for tangent in &tangents {
            assert!(tangent.iter().all(|value| value.is_finite()));
            assert!(
                (Vec3::from_array([tangent[0], tangent[1], tangent[2]]).length() - 1.0).abs()
                    < 1e-5
            );
            assert_eq!(tangent[3], 1.0);
        }
        let mirrored = [[0.0, 0.0], [-1.0, 0.0], [0.0, 1.0]];
        let mirrored_tangents = generate_tangents(&positions, &normals, &mirrored, &[0, 1, 2]);
        assert!(mirrored_tangents.iter().all(|tangent| tangent[3] == -1.0));
    }

    #[test]
    fn degenerate_uvs_receive_stable_finite_fallback_tangents() {
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let normals = [[0.0, 0.0, 1.0]; 3];
        let tangents = generate_tangents(&positions, &normals, &[[0.0; 2]; 3], &[0, 1, 2]);
        assert!(tangents.iter().flatten().all(|value| value.is_finite()));
        assert!(tangents.iter().all(|tangent| tangent[3] == 1.0));
    }

    #[test]
    fn legacy_upload_uses_static_layout_for_zero_weights_and_preserves_skinning_data() {
        let mut vertices = cube().vertices;
        assert!(!legacy_vertices_require_skinning(&vertices));

        vertices[0].weights[3] = 0.5;
        assert!(legacy_vertices_require_skinning(&vertices));

        vertices[0].weights[3] = f32::NAN;
        assert!(legacy_vertices_require_skinning(&vertices));
    }

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
