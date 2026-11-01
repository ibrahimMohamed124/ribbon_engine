# `ribbon_plugin_weather`

A reusable ECS weather simulation. The plugin owns particle creation, update, and recycling; game code
configures `RainSettings` at gameplay or level boundaries. Rain uses ordinary `Transform3D` and
`MeshInstance` components through `ribbon_render_contracts` and the native 3D mesh renderer.

The plugin must not depend on `ribbon_plugin_render3d` or any game package. Current rain is
visual-only: it has no collision, splash decals, wet-material simulation, puddles, or thunder audio.
