# engine_editor
Purpose: Immediate-mode panels drawn through `engine_render::DrawList`.
Allowed dependencies: app, plugins, platform, render, reflect, commands, scene, ECS, and math.
Provides layout, basic widgets, hit testing, selection, and the editor plugin shell.
Does not use egui or provide text input in v1.
