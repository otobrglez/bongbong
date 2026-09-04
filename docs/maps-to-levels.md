# Proposal for "maps to levels"

Purpose of this document is to introduce different objectives to each map. 
Objectives should also mean what player or enemies needs to do in other for the game to finish.

## Gameplay kinds

Map can have three different kinds of gameplay. Default one is "protect the frog"

### Kind: Protect the frog!

Players mission is to protect the frog from enemy tanks. If the frog dies, player looses the game.
If all enemies are successfuly destroyed, player wins. After winning player might proceed to next level/game but that is out of the scope for now.

### Kind: Hunt the frog!

Players mission is to kill enemies frog before enemies kill his frog. So the challenge is not to just protect but also attach opponents frog.
Whoever kills opponents frog first wins.

For this to work we need to support placing two frogs on map. Players and enemies.

### Kind: Destroy!

Player just needs to destroy all the enemy tanks. When all are destroyed he or she wins.

## Enemy spawning

### Default
By default game spawns random enemy tanks. Number of which is defined via CLI argument or via map attribute.
However, there should also be different spawning methods supported.

### Waves

Starting with lower class tanks (start/end class level can be configured). Come in waves. 
Each wave should bring more powerfull class of tank to battlefield.
Number of waves or total of enemy tanks that need to be destroyed in wave/total is configurable.
When wave comes tank should "roll-in" from outside of maps into map where there are no obsticles (walls etc).

## Acceptance criteria

- Enemy spawning is configurable via CLI and map (TOML)
- There should be a way to tune these parameters also via tuning mechanism
- Before game starts there should be big white overlay text saying "Protect the frog!" telling player what is the mission.

