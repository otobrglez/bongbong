# Housekeeping & Core Improvements for Future Growth

Based on the current architecture (v0.0.2) of BongBong, here is a breakdown of architectural improvements and housekeeping ideas to support future mechanics like walls, obstacles, collectables (awards), and more realistic physics.

## 1. Architectural Overhaul: Adopt an ECS (Entity Component System)
Currently, `game.rs` manually tracks vectors of different entities (`enemies`, `shells`, `tracks`, `impact_flashes`, etc.). As you add walls, destructible obstacles, and awards, this manual state management will become unwieldy.
*   **Action:** Introduce a lightweight ECS `hecs` - https://github.com/Ralith/hecs
*   **Benefits:** Instead of hardcoding `for enemy in &mut self.enemies`, you can query for any entity with a `Health`, `Position`, and `Collider` component. This makes adding an "award" as simple as spawning an entity with `Position`, `Collider` (sensor), and a `Collectable` component.

## 2. Decouple Game State from Rendering
`game.rs` currently mixes game state manipulation directly with Raylib rendering logic and textures. 
*   **Action:** Split the codebase into a pure simulation layer and a presentation layer. The simulation layer should only care about physics, logic, and state. 
*   **Benefits:** This will make unit testing much easier and allow you to cleanly pause, replay, or predict game states without bringing the renderer into it.

## 3. Realistic Tank Physics & Movement
The game currently forces "arcade" cardinal movement (tanks lock to 90-degree rotations, move at constant speeds, and come to an instant stop).
*   **Action:** Unlock the rigid bodies' rotations in Rapier2D and implement continuous (analog) tank driving.
*   **Mechanics:**
    *   **Thrust & Steering:** Instead of setting `linvel` directly, apply forces relative to the tank's forward vector.
    *   **Friction:** Use Rapier2D's anisotropic friction or manually apply lateral damping to simulate tank treads gripping the ground while allowing forward momentum.
    *   **Weight:** Different tank hulls can have different masses and moments of inertia, affecting their acceleration and turn rates.

## 4. Advanced Shell Physics & Ballistics
Shells are currently kinematic sensors that manually integrate position (`velocity * dt`). 
*   **Action:** Upgrade shells to be fully simulated dynamic bodies in Rapier2D (using Continuous Collision Detection - CCD).
*   **Mechanics:**
    *   **Bouncing & Ricochets:** By using physics materials with `restitution`, shells can bounce off certain metal obstacles or angled walls.
    *   **Impact Forces:** When a dynamic shell hits a tank, the physics engine can automatically resolve the kinetic energy transfer, causing realistic impacts and knockbacks without hand-rolling the math.
    *   **Collision Groups:** Implement Rapier's `InteractionGroups` (flagged as a pending task in `physics-engine-design.md`) to cleanly filter collisions so shells don't hit the tank that fired them, while allowing them to hit everything else.

## 5. Environment: Walls, Obstacles, and Awards
To move beyond a simple open arena with 4 bounding walls:
*   **Destructible Walls:** Spawn static rigid bodies with a custom `Health` component. When a shell's intersection event triggers on them, decrement health. When destroyed, remove the body and spawn rubble/dust effects.
*   **Awards & Power-ups:** Spawn static sensor colliders. Using the ECS pattern, a system can listen for intersection events between a player's collider and an award's sensor, granting buffs (e.g., fire rate, health repair) and despawning the award.
*   **Terrain Layouts:** Integrate a simple grid or tilemap system to define levels rather than spawning entities fully dynamically via random numbers.

## 6. AI Pathfinding
The AI currently uses predictive line-of-sight collision avoidance based on `Mover` structs. This works in an empty box but will fail spectacularly in a maze of walls and obstacles.
*   **Action:** Implement a pathfinding layer (such as A* on a grid or a NavMesh) that the AI can query to find routes around complex static obstacles. 

## Summary
The migration to Rapier2D was a fantastic first step. The next major leap in code health will be moving to an ECS, which will naturally facilitate adding the varied gameplay elements (obstacles, awards) you want. Following that, leaning fully into Rapier2D's dynamic bodies (unlocking rotation, using forces instead of setting velocities) will give you the realistic tank and shell physics you desire.
