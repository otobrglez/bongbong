//! What each kind of thing sounds like (docs/sound-system-prd.md section 4.4):
//! one exhaustive `match` per enumeration a sound depends on, so a new
//! chassis, weapon, material, pickup, sky, theme or biome fails to compile
//! until it has a sound decision (section 4.9). Each answers with a cue id
//! or a family that ids are built from; `audio_tests` walks every `ALL` and
//! checks the answer resolves in the manifest.

use crate::biome::Biome;
use crate::bong::BongKind;
use crate::map::{Theme, Weather};
use crate::obstacle::{Material, Surface as Face};
use crate::pickup::PickupKind;
use crate::simulation::{Against, HitCause};
use crate::tank::{ActiveWeapon, TankKind};
use crate::tower::TowerKind;
use crate::wear::{Look, Sky, Surface};
use crate::well::Swallow;

/// Which engine a chassis has: four families, each its own sample set,
/// pitched per chassis by its mass (`engine_pitch`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineFamily {
    Light,
    Medium,
    Heavy,
    Twin,
}

impl EngineFamily {
    pub fn cue(self) -> &'static str {
        match self {
            EngineFamily::Light => "engine.light",
            EngineFamily::Medium => "engine.medium",
            EngineFamily::Heavy => "engine.heavy",
            EngineFamily::Twin => "engine.twin",
        }
    }
}

pub fn engine_family(kind: TankKind) -> EngineFamily {
    match kind {
        TankKind::Scout | TankKind::Wraith | TankKind::Flak | TankKind::Glacier => EngineFamily::Light,
        TankKind::Assault | TankKind::Warden | TankKind::Longbow | TankKind::Obelisk => EngineFamily::Medium,
        TankKind::Breaker | TankKind::Ravager => EngineFamily::Heavy,
        TankKind::Titan | TankKind::Leviathan => EngineFamily::Twin,
    }
}

/// A chassis's engine pitch against its family's sample: heavier is lower.
/// `mass_factor` is the chassis's `tank_mass_factor` (0.77 to 1.89).
pub fn engine_pitch(mass_factor: f32) -> f32 {
    libm::powf(1.0 / mass_factor.max(0.1), 0.35)
}

/// How a cannon sounds by calibre (`tank_damage_factor`).
pub fn cannon_cue(damage_factor: f32) -> &'static str {
    if damage_factor < 0.85 {
        "weapon.shell.fire.light"
    } else if damage_factor < 1.1 {
        "weapon.shell.fire.medium"
    } else if damage_factor < 1.4 {
        "weapon.shell.fire.heavy"
    } else {
        "weapon.shell.fire.super"
    }
}

/// What firing a weapon sounds like at the trigger (`Event::Fired`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireSound {
    /// A one-shot at the tank.
    Cue(&'static str),
    /// The cannon, tiered by the chassis's calibre (`cannon_cue`).
    Cannon,
    /// A loop a watcher drives while the weapon fires.
    Watched(&'static str),
    /// Another event carries the sound: the weapon's own show.
    Elsewhere(&'static str),
}

pub fn weapon_sound(weapon: ActiveWeapon) -> FireSound {
    match weapon {
        ActiveWeapon::Shell => FireSound::Cannon,
        ActiveWeapon::Laser => FireSound::Elsewhere("LaserBeam"),
        ActiveWeapon::Plasma => FireSound::Cue("weapon.plasma.fire"),
        ActiveWeapon::Minigun => FireSound::Watched("minigun"),
        ActiveWeapon::Missiles => FireSound::Cue("weapon.missiles.fire"),
        ActiveWeapon::Flamethrower => FireSound::Watched("flame"),
        ActiveWeapon::Grenades => FireSound::Cue("weapon.grenades.fire"),
        ActiveWeapon::SonicHammer => FireSound::Elsewhere("SonicBlast"),
        ActiveWeapon::Emp => FireSound::Elsewhere("EmpPulse"),
        ActiveWeapon::GaussRail => FireSound::Elsewhere("RailSlug"),
        ActiveWeapon::FpvSwarm => FireSound::Elsewhere("DroneLaunched"),
        ActiveWeapon::RodFromGod => FireSound::Elsewhere("RodCalled"),
        ActiveWeapon::GravityWell => FireSound::Cue("weapon.gravity_well.fire"),
    }
}

/// What a surface sounds like when struck: the family part of a hit's cue
/// id (`hit.<shot>.<family>.<material>`).
pub fn surface_family(material: Material) -> &'static str {
    match material {
        Material::Brick | Material::Concrete | Material::Volcano | Material::Barrier => "masonry",
        Material::Iron | Material::Door | Material::Barrel | Material::Tesla | Material::GunTower | Material::TankTrap => "metal",
        Material::Wood | Material::Fence | Material::Target | Material::WoodCrate => "wood",
        Material::Glass | Material::Lamp => "glass",
        Material::Sandbag | Material::Hay => "earth",
        Material::Tyres => "rubber",
        Material::BioSlush => "slime",
        Material::Tree
        | Material::Pine
        | Material::Spruce
        | Material::Scots
        | Material::Fir
        | Material::Birch
        | Material::Willow
        | Material::Palm
        | Material::Snag => "foliage",
    }
}

/// The material's own name in a hit's cue id.
pub fn material_name(material: Material) -> &'static str {
    match material {
        Material::Brick => "brick",
        Material::Iron => "iron",
        Material::Wood => "wood",
        Material::Glass => "glass",
        Material::Concrete => "concrete",
        Material::Sandbag => "sandbag",
        Material::Barrel => "barrel",
        Material::Fence => "fence",
        Material::Tree => "tree",
        Material::Pine => "pine",
        Material::Tesla => "tesla",
        Material::GunTower => "gun_tower",
        Material::BioSlush => "bio_slush",
        Material::Volcano => "volcano",
        Material::Lamp => "lamp",
        Material::Door => "door",
        Material::Target => "target",
        Material::Spruce => "spruce",
        Material::Scots => "scots",
        Material::Fir => "fir",
        Material::Birch => "birch",
        Material::Willow => "willow",
        Material::Palm => "palm",
        Material::Snag => "snag",
        Material::Tyres => "tyres",
        Material::Hay => "hay",
        Material::WoodCrate => "wood_crate",
        Material::Barrier => "barrier",
        Material::TankTrap => "tank_trap",
    }
}

/// A hull's slam into a fixed face (`Event::HullStruck`), by how the face
/// takes it: iron, masonry and the towers crash, glass cracks, timber and
/// the firm trees thud, a birch or a willow creaks and shakes, the soft
/// props thump, a tyre stack throws the hull back with a rubber boing.
pub fn slam_cue(face: Face) -> &'static str {
    match face {
        Face::Plain | Face::Hardest | Face::Hard => "vehicle.slam",
        Face::Brittle => "vehicle.slam.glass",
        Face::Firm => "vehicle.slam.wood",
        Face::Yielding => "vehicle.slam.tree",
        Face::Soft => "vehicle.slam.soft",
        Face::Springy => "vehicle.slam.rubber",
    }
}

/// A struck hull's pitch against its cue's sample: heavier armour rings
/// lower, by `mass_factor` (`tank_mass_factor`) to the `exponent`
/// (`audio_hull_mass_pitch`).
pub fn hull_pitch(mass_factor: f32, exponent: f32) -> f32 {
    libm::powf(1.0 / mass_factor.max(0.1), exponent)
}

/// How loud a hit on a hull rings inside it, for the seat it struck
/// (`hit.hull_inside`), against the hit's own strength; `None` for what
/// does not ring the hull - a beam, a stream, a bolt of lightning, the
/// rod's crush and the world's burns.
pub fn hull_inside(cause: HitCause) -> Option<f32> {
    match cause {
        HitCause::Shell | HitCause::Rail => Some(1.0),
        HitCause::Sonic => Some(0.9),
        HitCause::Plasma => Some(0.8),
        HitCause::Well => Some(0.7),
        HitCause::Spall => Some(0.5),
        HitCause::Bullet => Some(0.35),
        HitCause::Laser | HitCause::Flame | HitCause::Tesla | HitCause::Rod | HitCause::Blast | HitCause::Lava | HitCause::Fire | HitCause::Slime => None,
    }
}

/// What a hull held against a face scrapes with (`Game::contacts_against`):
/// steel on another hull or a solid face, water slapping at deep water's
/// edge, nothing against lava's edge or the bong (`Surface::Plain`).
pub fn scrape_cue(against: Against) -> Option<&'static str> {
    match against {
        Against::Hull => Some("vehicle.scrape"),
        Against::DeepWater => Some("vehicle.scrape.water"),
        Against::Face(face) => match face {
            Face::Plain => None,
            Face::Hardest | Face::Hard | Face::Brittle | Face::Firm | Face::Yielding | Face::Soft | Face::Springy => Some("vehicle.scrape"),
        },
    }
}

/// The shot part of a hit's cue id.
pub fn hit_shot(cause: HitCause) -> &'static str {
    match cause {
        HitCause::Shell => "shell",
        HitCause::Bullet => "bullet",
        HitCause::Plasma => "plasma",
        HitCause::Laser => "laser",
        HitCause::Flame => "flame",
        HitCause::Tesla => "tesla",
        HitCause::Sonic => "sonic",
        HitCause::Rail => "rail",
        HitCause::Rod => "rod",
        HitCause::Well => "well",
        HitCause::Spall => "spall",
        HitCause::Blast => "blast",
        HitCause::Lava => "lava",
        HitCause::Fire => "fire",
        HitCause::Slime => "slime",
    }
}

/// What picking up a crate sounds like.
pub fn pickup_cue(kind: PickupKind) -> &'static str {
    match kind {
        PickupKind::Health | PickupKind::BongHealth => "pickup.health",
        PickupKind::Ammo => "pickup.ammo",
        PickupKind::SpeedUp | PickupKind::Shield | PickupKind::HeatShield => "pickup.boost",
        PickupKind::TowerPack => "pickup.tower",
        PickupKind::AirHeli | PickupKind::AirJet => "pickup.air",
        PickupKind::Laser
        | PickupKind::Minigun
        | PickupKind::Plasma
        | PickupKind::Missiles
        | PickupKind::Flamethrower
        | PickupKind::Grenades
        | PickupKind::SonicHammer
        | PickupKind::Emp
        | PickupKind::GaussRail
        | PickupKind::FpvSwarm
        | PickupKind::RodFromGod
        | PickupKind::GravityWell => "pickup.weapon",
    }
}

/// A bong's babble pitch: each kind its own voice (section 4.7).
pub fn bong_pitch(kind: BongKind) -> f32 {
    match kind {
        BongKind::Spike => 1.0,
        BongKind::Oggle => 0.85,
        BongKind::Gloop => 0.72,
        BongKind::Puff => 1.25,
        BongKind::Capi => 1.12,
        BongKind::Blip => 1.4,
        BongKind::Fuzz => 0.93,
    }
}

/// What a tower's shot sounds like.
pub fn tower_cue(kind: TowerKind) -> &'static str {
    match kind {
        TowerKind::Tesla => "tower.tesla.strike",
        TowerKind::Gun => "tower.gun.fire",
        TowerKind::Bio => "tower.bio.fire",
    }
}

/// What a gravity well swallowing something sounds like.
pub fn swallow_cue(what: Swallow) -> &'static str {
    match what {
        Swallow::Shell | Swallow::Bullet | Swallow::Plasma | Swallow::Orb | Swallow::Missile | Swallow::Drone => "weapon.gravity_well.swallow",
    }
}

/// The bed a sky lays under the round, if it has one.
pub fn sky_bed(weather: Weather) -> Option<&'static str> {
    match weather {
        Weather::Rain | Weather::Storm => Some("world.rain"),
        Weather::Sandstorm => Some("world.wind"),
        Weather::Snow => Some("world.wind.soft"),
        Weather::Night => Some("world.night"),
        Weather::Clear | Weather::Dusk | Weather::Fog | Weather::HeatHaze | Weather::Random => None,
    }
}

/// The bed a place lays under the round: the theme's, or its biome's.
pub fn place_bed(theme: Theme, biome: Option<Biome>) -> &'static str {
    match (theme, biome) {
        (_, Some(Biome::Valley)) => "bed.valley",
        (_, Some(Biome::Floodplain)) => "bed.floodplain",
        (_, Some(Biome::Karst)) => "bed.karst",
        (Theme::Grass, None) => "bed.grass",
        (Theme::Desert, None) => "bed.desert",
    }
}

/// The layer the tracks add for the ground under them.
pub fn surface_layer(surface: Surface) -> Option<&'static str> {
    match surface {
        Surface::Grass => Some("vehicle.ground.grass"),
        Surface::Road => None,
        Surface::Sand => Some("vehicle.ground.sand"),
        Surface::Ford => Some("vehicle.ground.water"),
        Surface::Deep => Some("vehicle.ground.water"),
        Surface::Ice => Some("vehicle.ground.ice"),
    }
}

/// The layer the tracks add for the ground under them as the round has it
/// (docs/ground-memory.md): lava for a hull wading it, snow under a snowy
/// sky, mud where the runs churn the ground to mud (`wear::muddy`), the
/// desert's pale dust where the tileset's grass is, and otherwise the
/// surface's own (`surface_layer`).
pub fn ground_layer(surface: Surface, look: Look, wet: f32, lava: bool) -> Option<&'static str> {
    if lava {
        return Some("vehicle.ground.lava");
    }
    match surface {
        Surface::Ford | Surface::Deep | Surface::Ice => surface_layer(surface),
        Surface::Grass | Surface::Road | Surface::Sand if look.sky == Sky::Snow => Some("vehicle.ground.snow"),
        Surface::Grass | Surface::Road | Surface::Sand if crate::wear::muddy(look, surface, wet) => Some("vehicle.ground.mud"),
        Surface::Grass if look.theme == Theme::Desert => Some("vehicle.ground.dust"),
        Surface::Grass | Surface::Road | Surface::Sand => surface_layer(surface),
    }
}

/// Tracks sliding across their runs or spinning on a slippery start
/// (`Roll`): a screech on ice, a slushy whirr in mud, a scrub on dry
/// ground. In water the wading layer covers it.
pub fn slip_cue(surface: Surface, muddy: bool) -> &'static str {
    match surface {
        Surface::Ice => "vehicle.slip.ice",
        Surface::Grass | Surface::Road | Surface::Sand | Surface::Ford | Surface::Deep if muddy => "vehicle.slip.mud",
        Surface::Grass | Surface::Road | Surface::Sand | Surface::Ford | Surface::Deep => "vehicle.slip",
    }
}

/// A hull knocked off its tracks sliding (`Game::skidding`), by what it
/// slides over: lava's gloop, a water wash, an ice hiss, snow, mud, a sand
/// hiss (the desert's dust too), torn turf, and steel scraping a road.
pub fn skid_cue(surface: Surface, look: Look, wet: f32, lava: bool) -> &'static str {
    if lava {
        return "vehicle.ground.lava";
    }
    match surface {
        Surface::Ford | Surface::Deep => "vehicle.skid.water",
        Surface::Ice => "vehicle.skid.ice",
        Surface::Grass | Surface::Road | Surface::Sand if look.sky == Sky::Snow => "vehicle.skid.snow",
        Surface::Grass | Surface::Road | Surface::Sand if crate::wear::muddy(look, surface, wet) => "vehicle.skid.mud",
        Surface::Grass if look.theme == Theme::Desert => "vehicle.skid.sand",
        Surface::Sand => "vehicle.skid.sand",
        Surface::Grass => "vehicle.skid.grass",
        Surface::Road => "vehicle.skid",
    }
}
