# Ribbon Editor Tools Plugin (`ribbon_plugin_editor_tools`)
Purpose: Hierarchy, reflected inspector, and runtime statistics panels.
Allowed dependencies: `ribbon_editor`, commands, reflect, scene, ECS, app, plugins, render, math, assets, streaming, and render3d.
Inspector changes run through undoable commands.
Does not add game-specific panels or gameplay behavior.


## Streaming Panel

The built-in debug editor shell also installs a read-only **Streaming** panel. It summarizes logical scenes and regions, active/loading/failed counts, AssetServer worker and staged-completion pressure, CPU residency/pinning estimates, source/decode timing from processed completions, and the render3d model-upload count/estimated-byte budget. It is diagnostic UI only: region authoring, spatial gizmos, per-asset dependency editing, and live budget controls are not part of this panel.
