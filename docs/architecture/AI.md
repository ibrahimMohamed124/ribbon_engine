# Gameplay AI

`ribbon_plugin_ai` contains generic ECS state machines and behavior trees, a typed blackboard, and a
single-layer XZ navigation grid. Grid paths use 8-neighbor A*, prevent diagonal corner cutting, honor
cell costs, cap expansions, and greedily smooth visible segments. `bake_nav_grid` samples physics
ground and clearance rays, then erodes walkable cells by agent radius.

`NavAgent` writes `NavSteering` each update. It never moves the entity; the game applies the desired
horizontal velocity to its controller. `Sight` computes range/FOV candidates and uses the physics
raycast result for visibility. Work is budgeted per update.

Not implemented: navmesh/Recast, multi-floor routing, dynamic obstacle avoidance or crowd steering,
hearing and memory perception, utility AI/GOAP, async path jobs, and automatic agent movement.
