# 0017 — Game-owned save sections

## Decision

`ribbon_save` owns the versioned file envelope, slot validation, CRC, backups, atomic replacement,
game identity checks, and section migration protocol. Games register typed `SaveSection` values and
own capture/apply semantics. Loading prepares every registered section before applying any of them.

RON is used for readable UTF-8 files. Saves use a data-directory `saves/` folder or a test override.
The first format version writes a temporary file, syncs it, preserves the previous main file as a
backup, then renames the new file into place. Event-driven save/load remains synchronous on the main
thread.

## Limits

There is no entity snapshot format, asynchronous I/O, compression, encryption, cloud sync, or
thumbnail capture. ATHAR saves room progress safely but does not restore a village scene.
