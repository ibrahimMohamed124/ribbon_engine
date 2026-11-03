# Architecture decisions

- [`0000-legacy-decision-log.md`](0000-legacy-decision-log.md) preserves the pre-ADR decision table unchanged.
- [`0001-game-owned-voxel-stack.md`](0001-game-owned-voxel-stack.md) — game/engine ownership boundary.
- [`0002-renderer-roles.md`](0002-renderer-roles.md) — renderer responsibilities.
- [`0003-input-leaf.md`](0003-input-leaf.md) — platform-neutral input layer.
- [`0004-profiling-leaf.md`](0004-profiling-leaf.md) — renderer-neutral profiling primitives.
- [`0005-athar-game-location.md`](0005-athar-game-location.md) — game directory ownership.
- [`0006-source-module-splits.md`](0006-source-module-splits.md) — large-file decomposition.
- [`0007-prefab-asset-codecs.md`](0007-prefab-asset-codecs.md) — context-aware asset path restoration.
- [`0008-profiling-csv-schema.md`](0008-profiling-csv-schema.md) — explicit game-only CSV schema trade-off.
- [`0009-athar-tool-ownership.md`](0009-athar-tool-ownership.md) — game-owned asset tools and path handling.
- [`0010-profiling-log-format.md`](0010-profiling-log-format.md) — exact capture completion log compatibility.

- [`0011-athar-village-restoration.md`](0011-athar-village-restoration.md) — restoration of the recoverable Athar village scene.
- [`0012-weather-render-contracts.md`](0012-weather-render-contracts.md) — reusable weather uses renderer-neutral data contracts.
- [`0013-contract-reflection-ownership.md`](0013-contract-reflection-ownership.md) — reflection implementations live beside contract-owned types.
- [`0014-app-states.md`](0014-app-states.md) — typed state resources, deferred transitions, and hooks.
- [`0015-gamepad-backend.md`](0015-gamepad-backend.md) — threaded gilrs polling and backend-neutral gamepad processing.
- [`0016-particle-system.md`](0016-particle-system.md) — engine particle simulation and renderer-owned particle draws.
- [`0017-save-system.md`](0017-save-system.md) — versioned game-owned save sections and atomic slot files.
- [`0018-ai-plugin.md`](0018-ai-plugin.md) — reusable FSM, behavior-tree, grid-navigation, and sight services.
