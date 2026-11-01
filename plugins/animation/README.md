# Ribbon Animation Plugin (`ribbon_plugin_animation`)

Provides deterministic sprite playback components and reusable CPU-side animation clip assets.

## Sprite clip assets

Register `SpriteAnimationClipLoader` with `AssetServer` and request `.ranim` files like any other
asset. `.ranim` uses RON and stores atlas frame indices, a positive finite frames-per-second value,
and whether the clip loops:

```ron
(
    name: "walk",
    frame_indices: [0, 1, 2, 3, 4, 5],
    frames_per_second: 12.0,
    looping: true,
)
```

Keep an `AssetRef<SpriteAnimationClip>` for as long as active players need the clip.
`SpriteClipPlayer` contains a generation-checked weak handle and advances without blocking while the
asset is not yet resident. The loader reports its decoded allocation estimate in the animation
category of `AssetServer::cpu_memory_stats`; this is estimated CPU residency, not allocator telemetry.
The sprite atlas texture remains a normal renderer resource and must be kept/warmed independently.

`SpriteAnimator` remains available for games that drive state/frame selection themselves.
