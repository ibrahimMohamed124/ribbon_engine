# Save system

`ribbon_save` stores versioned, CRC-checked RON envelopes in validated named slots. Games register
typed sections in a stable order and own what those sections capture and restore. The engine handles
game identity, format/version checks, migrations, atomic replacement, backups, listing, and deletion.

Loading parses and migrates every registered section before invoking any apply closure, preventing
partial application from missing sections or decode/migration failures. A corrupt current file can
fall back to its `.bak` copy. Saves run synchronously on the main thread.

ATHAR's `athar.progress` section records quest flags, door opening progress, player pose, and yaw.
Village saves currently restore room progress flags and return the player to the room start; full
village restoration is not implemented.

Not implemented: entity-level scene snapshots (games serialize their own state), async/background
I/O, compression, encryption/anti-tamper, cloud sync, screenshots, or thumbnails.
