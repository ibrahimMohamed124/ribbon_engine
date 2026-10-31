# Engine promotion candidates

This is an inventory only; no gameplay code is promoted by the architecture restructuring task. Re-evaluate these candidates separately after the game behavior is stable.

| Candidate | Current location | Why it may be reusable | Decision now |
|---|---|---|---|
| First-person camera/controller | `src/` controller and camera systems | Common input-to-camera behavior could serve other first-person games | Keep game-owned until a second consumer exists |
| Flashlight/view-model behavior | game scripts and render setup | Common attachment and light behavior could become a reusable plugin | Keep game-owned; preserve current room behavior |
| Horror atmosphere/audio setup | game scripts and assets | Reusable patterns, but content and tuning are game-specific | Keep game-owned |
