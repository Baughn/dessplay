//! Pure combat prose. Resolution supplies facts; callers supply the perceived
//! participants. Natural attacks never masquerade as their damage tool.
use super::{BodyKind, EnemyKind, Impact, Injury, InjuryReport, Organ, PartKind, WeaponKind};

#[derive(Clone, Copy)]
pub(super) enum AttackSource {
    Player(WeaponKind),
    Enemy(EnemyKind, WeaponKind),
    FallingStone,
}

#[derive(Clone, Copy)]
pub(super) enum Victim {
    Player,
    Enemy(EnemyKind),
}

impl Victim {
    fn possessive(self) -> String {
        match self {
            Self::Player => "your".into(),
            Self::Enemy(kind) => format!("the {}'s", kind.name()),
        }
    }

    fn part(self, part: PartKind) -> String {
        let body = match self {
            Self::Player => BodyKind::Human,
            Self::Enemy(kind) => kind.body_kind(),
        };
        format!("{} {}", self.possessive(), part.name(body))
    }
}

impl AttackSource {
    pub(super) fn death_source(self) -> &'static str {
        match self {
            Self::Player(_) => "your own attack",
            Self::Enemy(kind, _) => kind.name(),
            Self::FallingStone => "falling stone",
        }
    }

    fn action(self, target: &str, part: PartKind, deflected: bool) -> String {
        match self {
            Self::Player(weapon) => match (weapon, deflected) {
                (WeaponKind::Unarmed, false) => format!("You strike {target}"),
                (WeaponKind::Knife, false) => format!("You cut {target} with your knife"),
                (WeaponKind::Spear, false) => format!("You drive your spear into {target}"),
                (WeaponKind::Mace, false) => format!("You bring your mace down on {target}"),
                (WeaponKind::Unarmed, true) => format!("You strike at {target}"),
                (WeaponKind::Knife, true) => format!("You slash at {target} with your knife"),
                (WeaponKind::Spear, true) => format!("You thrust your spear at {target}"),
                (WeaponKind::Mace, true) => format!("You swing your mace at {target}"),
            },
            Self::Enemy(EnemyKind::Rat, _) => {
                if matches!(
                    part,
                    PartKind::Head
                        | PartKind::Torso
                        | PartKind::LeftArm
                        | PartKind::RightArm
                        | PartKind::LeftHand
                        | PartKind::RightHand
                ) {
                    format!("The ash rat jumps up to bite {target}")
                } else if deflected {
                    format!("The ash rat snaps at {target}")
                } else {
                    format!("The ash rat bites {target}")
                }
            }
            Self::Enemy(EnemyKind::Brute, _) => {
                format!("The cavern brute brings its weight down on {target}")
            }
            Self::Enemy(kind, weapon) => {
                let action = match (weapon, deflected) {
                    (WeaponKind::Unarmed, false) => "strikes",
                    (WeaponKind::Knife, false) => "cuts",
                    (WeaponKind::Spear, false) => "drives its spear into",
                    (WeaponKind::Mace, false) => "brings its mace down on",
                    (WeaponKind::Unarmed, true) => "lashes out at",
                    (WeaponKind::Knife, true) => "slashes at",
                    (WeaponKind::Spear, true) => "thrusts its spear at",
                    (WeaponKind::Mace, true) => "swings its mace at",
                };
                let tool = if weapon == WeaponKind::Knife {
                    " with its knife"
                } else {
                    ""
                };
                format!("The {} {action} {target}{tool}", kind.name())
            }
            Self::FallingStone => format!("Falling stone strikes {target}"),
        }
    }
}

impl InjuryReport {
    pub(super) fn narrate(&self, source: AttackSource, victim: Victim) -> String {
        let (part, consequences, deflected) = match &self.impact {
            Impact::AlreadyDead => {
                return match victim {
                    Victim::Player => "Your body is already still.".into(),
                    Victim::Enemy(kind) => format!("The {} is already still.", kind.name()),
                };
            }
            Impact::Deflected(part) => (*part, &[][..], true),
            Impact::Hit { part, consequences } => (*part, consequences.as_slice(), false),
        };
        let target = victim.part(part);
        let mut text = source.action(&target, part, deflected);
        if deflected {
            text.push_str(&format!(
                ", but {} armor turns the impact aside.",
                victim.possessive()
            ));
        } else {
            text.push('.');
        }
        for consequence in consequences {
            let detail = match *consequence {
                Injury::Fracture => format!("The bone in {target} breaks."),
                Injury::Severed => format!("{target} is severed."),
                Injury::Organ { organ, destroyed } => {
                    let state = if destroyed { "destroyed" } else { "damaged" };
                    let effect = match organ {
                        Organ::LeftEye | Organ::RightEye => " Sight suffers.",
                        Organ::LeftLung | Organ::RightLung => " Breathing grows harder.",
                        _ => "",
                    };
                    format!(
                        "{} {} is {state}.{effect}",
                        victim.possessive(),
                        organ.name()
                    )
                }
            };
            text.push(' ');
            let mut chars = detail.chars();
            if let Some(first) = chars.next() {
                text.extend(first.to_uppercase());
                text.push_str(chars.as_str());
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::roguelike::{
        ArmorMaterial, ArmorPiece, ArmorSlot, AttackProfile, Body, Equipment, Rng,
    };

    #[test]
    fn real_impacts_keep_their_target_consequences_and_ownership() {
        let mut saw_deflection = false;
        let mut saw_severance = false;
        let mut saw_organ = false;
        let mut targets = [false; 10];
        for kind in [
            EnemyKind::Rat,
            EnemyKind::Hollow,
            EnemyKind::Warden,
            EnemyKind::Brute,
        ] {
            for seed in 0..256 {
                let mut body = Body::new(kind.body_kind());
                let mut rng = Rng(seed);
                let mut gear = Equipment {
                    armor: [None; 6],
                    ..Equipment::default()
                };
                let power = if seed % 2 == 0 {
                    gear.armor = ArmorSlot::ALL.map(|slot| {
                        Some(ArmorPiece {
                            slot,
                            material: ArmorMaterial::Iron,
                        })
                    });
                    0
                } else {
                    60
                };
                let report = body.hit(
                    AttackProfile {
                        weapon: WeaponKind::Spear,
                        power,
                    },
                    &gear,
                    &mut rng,
                );
                let before = (body.clone(), rng.clone());
                for source in [
                    AttackSource::Player(WeaponKind::Spear),
                    AttackSource::FallingStone,
                ] {
                    let text = report.narrate(source, Victim::Enemy(kind));
                    let part = match &report.impact {
                        Impact::AlreadyDead => panic!("fresh body"),
                        Impact::Deflected(part) => {
                            saw_deflection = true;
                            assert!(text.contains("armor turns the impact aside"), "{text}");
                            assert!(!report.serious);
                            *part
                        }
                        Impact::Hit { part, consequences } => {
                            for consequence in consequences {
                                match consequence {
                                    Injury::Fracture => assert!(text.contains("breaks."), "{text}"),
                                    Injury::Severed => {
                                        saw_severance = true;
                                        assert!(text.contains("is severed."), "{text}");
                                    }
                                    Injury::Organ { organ, destroyed } => {
                                        saw_organ = true;
                                        let state =
                                            if *destroyed { "destroyed" } else { "damaged" };
                                        assert!(
                                            text.contains(&format!(
                                                "{}'s {} is {state}",
                                                kind.name(),
                                                organ.name()
                                            )),
                                            "{text}"
                                        );
                                    }
                                }
                            }
                            *part
                        }
                    };
                    targets[part.index()] = true;
                    assert!(
                        text.contains(&format!(
                            "{}'s {}",
                            kind.name(),
                            part.name(kind.body_kind())
                        )),
                        "{text}"
                    );
                    assert!(
                        !text.contains(&format!("your {}", part.name(kind.body_kind()))),
                        "{text}"
                    );
                }
                assert_eq!((body, rng), before, "narration is observation only");
            }
        }
        assert!(saw_deflection && saw_severance && saw_organ && targets.into_iter().all(|hit| hit));
    }

    #[test]
    fn every_rat_target_has_an_appropriate_bite_including_deflections() {
        for part in PartKind::ALL {
            for deflected in [false, true] {
                let report = InjuryReport {
                    impact: if deflected {
                        Impact::Deflected(part)
                    } else {
                        Impact::Hit {
                            part,
                            consequences: Vec::new(),
                        }
                    },
                    serious: false,
                };
                let text = report.narrate(
                    AttackSource::Enemy(EnemyKind::Rat, WeaponKind::Knife),
                    Victim::Player,
                );
                assert!(
                    text.contains(&format!("your {}", part.name(BodyKind::Human))),
                    "{text}"
                );
                assert!(!text.contains("knife"));
                if part.index() < 6 {
                    assert!(
                        text.starts_with("The ash rat jumps up to bite your "),
                        "{text}"
                    );
                } else {
                    assert!(!text.contains("jumps"), "{text}");
                }
                assert_eq!(text.contains("armor"), deflected);
            }
        }
    }
}
