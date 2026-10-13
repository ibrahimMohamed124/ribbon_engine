# STUDS — Horror Audio Pack

These are original, procedurally synthesized sound effects created for this project.
They do not contain third-party samples. You may use, modify, and redistribute these
generated WAV files as part of STUDS, including in commercial builds.

## Files
- `footstep.wav` — short, dampened gritty footstep. Trigger per step; avoid playing every frame.
- `jump.wav` — airy jump / movement whoosh.
- `page_collect.wav` — paper flutter plus eerie glassy pickup chime.
- `enemy_ambient.wav` — 8-second low, unsettling drone. Designed to loop; keep quieter than effects.
- `enemy_sting.wav` — short dissonant sting for a reveal or close encounter. Do not spam it.
- `victory.wav` — restrained, ominous resolved chord for the win state.
- `death.wav` — low impact and descending tail for the death / game-over state.
- `menu_ambience.wav` — optional 10-second low wind drone for title/menu screens.

## Format
- WAV, PCM 16-bit, 44.1 kHz.
- Mono for one-shot effects; stereo for ambience.
- The game uses Rodio for audio output; WAV decoding is enabled in its dependency configuration.
- Suggested mix: ambience 10–20%, footsteps 25–40%, pickup 50–70%, enemy sting 55–75%, win/death 60–80%.
  Tune these values against the game's actual output; they are starting points, not measured loudness targets.

## Integration
The game embeds the gameplay WAV files in the executable and plays them through Rodio. One-shot effects are triggered by gameplay events. Footsteps play by distance traveled, not every frame. `enemy_ambient.wav` loops after the enemy wakes and its volume rises with proximity. Only ambience is looped.

`menu_ambience.wav` loops in the main menu and menu screens. If the system has no audio output device, the game continues silently.

Linux builds require ALSA development headers (`libasound2-dev` on Debian/Ubuntu, `alsa-lib-devel` on Fedora).
