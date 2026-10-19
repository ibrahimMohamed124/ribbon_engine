# engine_commands
Purpose: Reversible component edits and bounded undo/redo history.
Allowed dependencies: `engine_ecs`, `engine_reflect`, and `engine_scene`.
Provides grouped commands, reflected field editing, and component add/remove.
Does not own editor UI or persist scenes.
