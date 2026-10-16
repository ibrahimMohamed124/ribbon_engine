# GPU shader port checklist

The eventual wgpu backend must preserve the CPU reference renderer's behavior
for these stages:

- voxel DDA cast and world-boundary behavior
- `face_ao` neighborhood sampling and interpolation
- entity box tests, model bounds, and phase-driven transforms
- flashlight cone, distance falloff, fog, and entity lighting
- tone mapping, display gamma, and palette lookup table
- vignette, frame grain, and ordered Bayer dithering
- procedural brick/noise texture sampling and distance fade

No shader source or GPU dependency is included yet.
