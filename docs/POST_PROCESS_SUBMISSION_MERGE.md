# Post-processing command submission merge

`ToneMapper::render` formerly created and submitted one command buffer for the bloom pass, then created and submitted a second command buffer for tone mapping. It now records both ordered render passes in the same encoder and submits once.

This change reduces the number of post-processing queue submissions from two to one per call. It does not guarantee a frame-time improvement; verify with the existing profile capture workflow. Other clear submissions and renderer call sites are intentionally unchanged because they serve additional window/offscreen paths and have not been proven redundant across all paths.
