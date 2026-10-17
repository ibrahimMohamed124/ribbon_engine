# engine_scene
Provides transform and hierarchy components, reflected prefab capture/instantiation, and RON scene documents.
Allowed workspace dependencies: `engine_ecs`, `engine_reflect`, `engine_math`, `engine_app`, and `engine_plugins`.
Allowed external dependencies: `serde` and `ron`.
It does not own render data, asset loading, or game-specific components.
