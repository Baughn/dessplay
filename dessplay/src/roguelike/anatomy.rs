//! Shared creature anatomy. Ordinary care stabilizes damage; only a fountain
//! restores structural tissue. No presentation clock or random source lives here.

use serde::{Deserialize, Serialize};

use super::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Species determines tissue durability and innate perception.
pub enum BodyKind {
    /// Human explorer or hollow remnant anatomy.
    Human,
    /// Fragile scavenger anatomy with small physiological tolerance.
    Rat,
    /// Thick cavern anatomy with poor natural vision.
    Brute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Stable region identifiers used by injury reports and treatment selection.
pub enum PartKind {
    /// Head, including eyes and the protected brain.
    Head,
    /// Chest and abdomen, including heart and lungs.
    Torso,
    /// Left arm or forelimb supporting its extremity.
    LeftArm,
    /// Right arm or forelimb supporting its extremity.
    RightArm,
    /// Left gripping extremity.
    LeftHand,
    /// Right gripping extremity.
    RightHand,
    /// Left leg or hindlimb supporting its foot.
    LeftLeg,
    /// Right leg or hindlimb supporting its foot.
    RightLeg,
    /// Left weight-bearing extremity.
    LeftFoot,
    /// Right weight-bearing extremity.
    RightFoot,
}

impl PartKind {
    /// All variants in their stable storage and inspection order.
    pub const ALL: [Self; 10] = [
        Self::Head,
        Self::Torso,
        Self::LeftArm,
        Self::RightArm,
        Self::LeftHand,
        Self::RightHand,
        Self::LeftLeg,
        Self::RightLeg,
        Self::LeftFoot,
        Self::RightFoot,
    ];

    /// Short player-facing name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Torso => "torso",
            Self::LeftArm => "left arm",
            Self::RightArm => "right arm",
            Self::LeftHand => "left hand",
            Self::RightHand => "right hand",
            Self::LeftLeg => "left leg",
            Self::RightLeg => "right leg",
            Self::LeftFoot => "left foot",
            Self::RightFoot => "right foot",
        }
    }

    /// Equipment slot protecting this anatomical region.
    pub fn armor_slot(self) -> ArmorSlot {
        match self {
            Self::Head => ArmorSlot::Head,
            Self::Torso => ArmorSlot::Torso,
            Self::LeftArm | Self::RightArm => ArmorSlot::Arms,
            Self::LeftHand | Self::RightHand => ArmorSlot::Hands,
            Self::LeftLeg | Self::RightLeg => ArmorSlot::Legs,
            Self::LeftFoot | Self::RightFoot => ArmorSlot::Feet,
        }
    }

    fn child(self) -> Option<usize> {
        match self {
            Self::LeftArm => Some(4),
            Self::RightArm => Some(5),
            Self::LeftLeg => Some(8),
            Self::RightLeg => Some(9),
            _ => None,
        }
    }
}

/// Integrity values use species-specific maxima. `severed` also means the
/// distal part is absent; a detached hand cannot continue holding a weapon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// The tissues and treatment state of one anatomical region.
pub struct BodyPart {
    /// Species or anatomical region represented by this value.
    pub kind: PartKind,
    /// Remaining superficial tissue, bounded by the species maximum.
    pub flesh: u16,
    /// Remaining structural support; ordinary care never restores it.
    pub bone: u16,
    /// Remaining nerve function as a percentage; damage is lasting.
    pub nerve: u16,
    /// Blood lost from this region per 100 simulation time units.
    pub bleeding: u16,
    /// Whether a finite splint currently supports a fractured region.
    pub splinted: bool,
    /// Whether this region is absent; distal anatomy is absent with it.
    pub severed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Complete persistent physiology and anatomy of one living creature.
pub struct Body {
    /// Species or anatomical region represented by this value.
    pub kind: BodyKind,
    /// Circulating blood, from zero (fatal) to 1000.
    pub blood: u16,
    /// Available breath, from zero to the current lung-dependent capacity.
    pub stamina: u16,
    /// Nutrition reserve from zero to 100; higher values mean better nourishment.
    pub hunger: u16,
    /// Anatomical regions in the stable order of `PartKind::ALL`.
    pub parts: [BodyPart; 10],
    /// Left and right eye integrity percentages; zero means permanent blindness.
    pub eyes: [u16; 2],
    /// Brain integrity percentage; destruction is fatal.
    pub brain: u16,
    /// Heart integrity percentage; destruction is fatal.
    pub heart: u16,
    /// Left and right lung integrity percentages; these limit breath capacity.
    pub lungs: [u16; 2],
    nutrition_clock: u8,
}

impl Default for Body {
    fn default() -> Self {
        Self::new(BodyKind::Human)
    }
}

impl Body {
    /// Construct an uninjured body with full physiological reserves.
    pub fn new(kind: BodyKind) -> Self {
        let (flesh, bone) = Self::durability(kind);
        Self {
            kind,
            blood: 1000,
            stamina: 100,
            hunger: 100,
            parts: PartKind::ALL.map(|kind| BodyPart {
                kind,
                flesh,
                bone,
                nerve: 100,
                bleeding: 0,
                splinted: false,
                severed: false,
            }),
            eyes: [100; 2],
            brain: 100,
            heart: 100,
            lungs: [100; 2],
            nutrition_clock: 0,
        }
    }

    fn durability(kind: BodyKind) -> (u16, u16) {
        match kind {
            BodyKind::Human => (80, 100),
            BodyKind::Rat => (20, 25),
            BodyKind::Brute => (130, 150),
        }
    }

    /// Total continuing blood loss per physiology interval.
    pub fn bleeding(&self) -> u16 {
        self.parts.iter().map(|p| p.bleeding).sum()
    }

    /// Current pain from superficial and lasting injuries, capped at 100.
    pub fn pain(&self) -> u16 {
        let (flesh, bone) = Self::durability(self.kind);
        self.parts
            .iter()
            .map(|p| {
                if p.severed {
                    7
                } else {
                    (flesh - p.flesh) / 5
                        + (bone - p.bone) / if p.splinted { 15 } else { 8 }
                        + (100 - p.nerve) / 20
                }
            })
            .sum::<u16>()
            .min(100)
    }

    /// Whether blood loss or destroyed vital anatomy prevents continued life.
    pub fn is_dead(&self) -> bool {
        self.blood == 0
            || self.brain == 0
            || self.heart == 0
            || self.lungs == [0, 0]
            || self.parts[0].severed
    }

    /// Describe the lethal condition, prioritizing destroyed vital anatomy.
    pub fn death_cause(&self) -> &'static str {
        if self.parts[0].severed {
            "decapitation"
        } else if self.brain == 0 {
            "a destroyed brain"
        } else if self.heart == 0 {
            "a ruptured heart"
        } else if self.lungs == [0, 0] {
            "destroyed lungs"
        } else if self.blood == 0 {
            "blood loss"
        } else {
            "mortal injuries"
        }
    }

    /// Current sight radius, including eye loss and species limitations.
    pub fn vision_radius(&self) -> i32 {
        let sight = self.eyes[0].saturating_add(self.eyes[1]);
        let radius = match sight {
            0 => 1,
            1..=49 => 2,
            50..=99 => 3,
            100..=149 => 5,
            _ => 7,
        };
        if self.kind == BodyKind::Brute {
            radius.min(2)
        } else {
            radius
        }
    }

    /// Maximum breath allowed by the remaining functional lungs.
    pub fn breath_capacity(&self) -> u16 {
        // Validation also calls this method on freshly deserialized data.
        ((u32::from(self.lungs[0]) + u32::from(self.lungs[1])) / 2).clamp(10, 100) as u16
    }

    /// One whole simulation interval, regardless of the action that spent it.
    pub fn tick(&mut self) {
        if self.is_dead() {
            return;
        }
        self.blood = self.blood.saturating_sub(self.bleeding());
        self.nutrition_clock += 1;
        if self.nutrition_clock == 20 {
            self.nutrition_clock = 0;
            self.hunger = self.hunger.saturating_sub(1);
        }
        if self.hunger == 0 {
            self.blood = self.blood.saturating_sub(2);
        }
        self.stamina = self.stamina.min(self.breath_capacity());
    }

    /// Waiting catches breath; it does not advance time or repair injuries.
    pub fn wait(&mut self) {
        if !self.is_dead() {
            let gain = (14_u16.saturating_sub(self.pain() / 10)).max(3);
            self.stamina = self
                .stamina
                .saturating_add(gain)
                .min(self.breath_capacity());
        }
    }

    fn part_function(&self, index: usize) -> u16 {
        let p = &self.parts[index];
        if p.severed {
            return 0;
        }
        let (_, maximum) = Self::durability(self.kind);
        let support = if p.splinted { 35 } else { 0 };
        ((p.bone * 100 / maximum).saturating_add(support).min(100)).min(p.nerve)
    }

    /// Count hands with sufficient arm, bone, and nerve function to grip.
    pub fn usable_hands(&self) -> usize {
        [(2, 4), (3, 5)]
            .iter()
            .filter(|(arm, hand)| self.part_function(*arm).min(self.part_function(*hand)) >= 20)
            .count()
    }

    /// Gear remains carried after injury, but attacks always have a viable fallback.
    pub fn effective_weapon(&self, gear: &Equipment) -> WeaponKind {
        if self.can_wield(gear.active) {
            gear.active
        } else {
            WeaponKind::Unarmed
        }
    }

    /// Whether the remaining hands can use this weapon; unarmed is always available.
    pub fn can_wield(&self, weapon: WeaponKind) -> bool {
        match weapon {
            WeaponKind::Unarmed => true,
            WeaponKind::Knife | WeaponKind::Mace => self.usable_hands() >= 1,
            WeaponKind::Spear => self.usable_hands() >= 2,
        }
    }

    /// Apply grip, pain, and exhaustion penalties while retaining a weak fallback.
    pub fn attack_power(&self, base: u16) -> u16 {
        let grip = self
            .part_function(2)
            .min(self.part_function(4))
            .max(self.part_function(3).min(self.part_function(5)));
        let multiplier = (50 + grip / 2).saturating_sub(self.pain() / 3);
        let exhausted = if self.stamina < 10 { 60 } else { 100 };
        ((u32::from(base) * u32::from(multiplier) * exhausted / 10_000) as u16).max(1)
    }

    /// Action duration in multiples of fifty, including gait and carried load.
    pub fn movement_cost(&self, sprint: bool, gear: &Equipment) -> u64 {
        let left = self.part_function(6).min(self.part_function(8));
        let right = self.part_function(7).min(self.part_function(9));
        let mobility = left + right;
        let penalty = match mobility {
            0..=39 => 200, // crawling remains possible
            40..=99 => 100,
            100..=159 => 50,
            _ => 0,
        };
        let load = if gear.weight() >= 28 { 50 } else { 0 };
        (if sprint { 50 } else { 100 }) + penalty + load
    }

    /// Breath required to sprint one tile with this body and equipment.
    pub fn sprint_cost(&self, gear: &Equipment) -> u16 {
        9 + gear.weight() / 2
            + self.pain() / 10
            + (200 - self.part_function(6) - self.part_function(7)) / 20
    }

    /// Bind the chosen bleeding region, or the most urgent one when unspecified.
    pub fn bandage(&mut self, supplies: &mut Supplies, target: Option<usize>) -> CareResult {
        if self.is_dead() {
            return CareResult::unchanged("The dead cannot bind their wounds.");
        }
        let index = match target {
            Some(i) if i < self.parts.len() => i,
            Some(_) => return CareResult::unchanged("There is no such wound."),
            None => self
                .parts
                .iter()
                .enumerate()
                .max_by_key(|(_, p)| p.bleeding)
                .map_or(0, |(i, _)| i),
        };
        let part = &mut self.parts[index];
        if part.bleeding == 0 {
            return CareResult::unchanged("That wound is not bleeding.");
        }
        if supplies.bandages == 0 {
            return CareResult::unchanged("You have no clean linen left.");
        }
        supplies.bandages -= 1;
        part.bleeding = 0;
        CareResult::changed(format!(
            "You bind the {}. The bleeding stops.",
            part.kind.name()
        ))
    }

    /// Spend one ration when hungry enough to benefit.
    pub fn eat(&mut self, supplies: &mut Supplies) -> CareResult {
        if self.is_dead() {
            return CareResult::unchanged("The dead cannot eat.");
        }
        if self.hunger > 75 {
            return CareResult::unchanged("You are not hungry enough to eat.");
        }
        if supplies.food == 0 {
            return CareResult::unchanged("You have no food left.");
        }
        supplies.food -= 1;
        self.hunger = (self.hunger + 50).min(100);
        CareResult::changed("You eat dried apples and hard bread.")
    }

    fn splint_target(&self) -> Option<usize> {
        self.parts
            .iter()
            .enumerate()
            .filter(|(i, _)| self.needs_splint(*i))
            .min_by_key(|(_, p)| p.bone)
            .map(|(i, _)| i)
    }

    fn needs_splint(&self, index: usize) -> bool {
        let (_, maximum) = Self::durability(self.kind);
        self.parts.get(index).is_some_and(|part| {
            index >= 2 && !part.severed && !part.splinted && part.bone < maximum * 3 / 4
        })
    }

    fn splint(&mut self, supplies: &mut Supplies, index: usize) -> CareResult {
        if !self.needs_splint(index) {
            return CareResult::unchanged("That injury cannot benefit from a splint.");
        }
        if supplies.splints == 0 {
            return CareResult::unchanged("You have no splints left.");
        }
        self.parts[index].splinted = true;
        supplies.splints -= 1;
        CareResult::changed(format!(
            "You splint your {}. The fracture is supported, not healed.",
            self.parts[index].kind.name()
        ))
    }

    /// Manual treatment stays local to the selected condition row. It follows
    /// the same priorities as automatic care without silently choosing another
    /// limb, and can support a fracture even when linen has run out.
    pub fn treat(&mut self, supplies: &mut Supplies, index: usize) -> CareResult {
        if self.is_dead() {
            return CareResult::unchanged("The dead cannot tend their wounds.");
        }
        let Some(part) = self.parts.get(index) else {
            return CareResult::unchanged("There is no such wound.");
        };
        if part.bleeding > 0 && supplies.bandages > 0 {
            return self.bandage(supplies, Some(index));
        }
        if self.needs_splint(index) {
            return self.splint(supplies, index);
        }
        if part.bleeding > 0 {
            return CareResult::unchanged("You have no clean linen left.");
        }
        CareResult::unchanged("No further treatment can help this injury now.")
    }

    /// Whether available supplies, rest, or ordinary care can still improve this body.
    pub fn can_recover(&self, supplies: &Supplies) -> bool {
        !self.is_dead()
            && ((self.bleeding() > 0 && supplies.bandages > 0)
                || (self.splint_target().is_some() && supplies.splints > 0)
                || (self.hunger <= 50 && supplies.food > 0)
                || self.stamina < self.breath_capacity()
                || (self.hunger > 0
                    && self.bleeding() == 0
                    && (self.blood < 1000
                        || self
                            .parts
                            .iter()
                            .any(|p| !p.severed && p.flesh < Self::durability(self.kind).0))))
    }

    /// Perform one automatic treatment or recovery step without advancing time.
    pub fn care_step(&mut self, supplies: &mut Supplies) -> CareResult {
        if self.is_dead() {
            return CareResult::unchanged("The dead cannot rest.");
        }
        if self.bleeding() > 0 && supplies.bandages > 0 {
            return self.bandage(supplies, None);
        }
        if supplies.splints > 0
            && let Some(index) = self.splint_target()
        {
            return self.splint(supplies, index);
        }
        if self.hunger <= 50 && supplies.food > 0 {
            return self.eat(supplies);
        }
        let before = self.clone();
        self.wait();
        if self.hunger > 0 && self.bleeding() == 0 {
            self.blood = (self.blood + 6).min(1000);
            let maximum = Self::durability(self.kind).0;
            for part in &mut self.parts {
                if !part.severed {
                    part.flesh = (part.flesh + 1).min(maximum);
                }
            }
        }
        if *self != before {
            CareResult::changed("You catch your breath and tend what can recover.")
        } else if self.bleeding() > 0 {
            CareResult::unchanged(
                "Without linen you cannot stop the bleeding. Rest cannot help further.",
            )
        } else {
            CareResult::unchanged("Further rest cannot mend the damage that remains.")
        }
    }

    /// Miraculously restore all tissues and reserves, retaining the species.
    pub fn restore(&mut self) {
        *self = Self::new(self.kind);
    }

    /// Weighted region then armor coverage and tissue penetration. All rolls
    /// come from the expedition RNG and therefore survive an interrupted fight.
    pub fn hit(&mut self, profile: AttackProfile, gear: &Equipment, rng: &mut Rng) -> InjuryReport {
        if self.is_dead() {
            return InjuryReport {
                message: "The body is already still.".into(),
                serious: false,
            };
        }
        // Duplicate torso/limb slots reflect surface area without hidden aim bonuses.
        const TARGETS: [usize; 20] = [0, 0, 1, 1, 1, 1, 1, 1, 2, 2, 3, 3, 4, 5, 6, 6, 7, 7, 8, 9];
        let mut index = TARGETS[rng.below(TARGETS.len() as u64) as usize];
        if self.parts[index].severed {
            index = 1;
        }
        let armor_roll = rng.below(100) as u16;
        let spread = rng.below(9) as u16;
        let organ_roll = rng.below(100) as u16;
        let armor = gear.armor[self.parts[index].kind.armor_slot().index()];
        let protection = armor
            .filter(|p| armor_roll < p.coverage())
            .map_or(0, |p| p.protection(profile.weapon));
        let impact = profile
            .power
            .saturating_add(spread)
            .saturating_sub(protection)
            .min(1000);
        if impact == 0 {
            return InjuryReport {
                message: format!(
                    "Armor turns the blow from the {}.",
                    self.parts[index].kind.name()
                ),
                serious: false,
            };
        }
        let (max_flesh, max_bone) = Self::durability(self.kind);
        let part = &mut self.parts[index];
        let old_bone = part.bone;
        let flesh_damage = match profile.weapon {
            WeaponKind::Mace => impact / 2 + 1,
            WeaponKind::Spear => impact * 3 / 4 + 1,
            _ => impact,
        };
        part.flesh = part.flesh.saturating_sub(flesh_damage);
        let penetration = match profile.weapon {
            WeaponKind::Mace => impact * 2 / 3,
            WeaponKind::Spear => impact / 2,
            _ => impact / 4,
        };
        // Repeated wounds expose deeper layers; blunt trauma crosses intact flesh.
        let exposed = part.flesh < max_flesh / 2;
        let bone_damage = penetration + if exposed { impact / 3 } else { 0 };
        part.bone = part.bone.saturating_sub(bone_damage);
        if part.bone < max_bone / 2 {
            part.nerve = part.nerve.saturating_sub(impact / 3);
        }
        let bleed = match profile.weapon {
            WeaponKind::Mace | WeaponKind::Unarmed => impact / 12,
            WeaponKind::Knife => impact / 4 + 1,
            WeaponKind::Spear => impact / 5 + 1,
        };
        part.bleeding = part.bleeding.saturating_add(bleed).min(100);
        // Small bodies cannot lose a human amount of blood before dying.
        let blood_scale = match self.kind {
            BodyKind::Rat => 14,
            BodyKind::Human => 2,
            BodyKind::Brute => 1,
        };
        self.blood = self
            .blood
            .saturating_sub(impact.saturating_mul(blood_scale));
        let fractured = old_bone >= max_bone * 3 / 4 && part.bone < max_bone * 3 / 4;
        let mut consequence = if fractured {
            " Bone breaks.".to_owned()
        } else {
            String::new()
        };
        let mut serious = fractured || impact >= 35;
        if part.flesh == 0
            && part.bone == 0
            && matches!(profile.weapon, WeaponKind::Knife | WeaponKind::Spear)
            && index != 1
        {
            part.severed = true;
            part.nerve = 0;
            part.splinted = false;
            part.bleeding = 35;
            consequence = " It is severed.".into();
            serious = true;
            if let Some(child) = part.kind.child() {
                self.parts[child].flesh = 0;
                self.parts[child].bone = 0;
                self.parts[child].nerve = 0;
                self.parts[child].bleeding = 0;
                self.parts[child].splinted = false;
                self.parts[child].severed = true;
            }
        }
        if index == 0 && organ_roll < 30 {
            let eye = (organ_roll % 2) as usize;
            let old = self.eyes[eye];
            self.eyes[eye] = old.saturating_sub(impact.saturating_mul(3));
            consequence.push_str(if self.eyes[eye] == 0 {
                " An eye is destroyed."
            } else {
                " An eye is injured; sight narrows."
            });
            serious = true;
        }
        if index == 0 && self.parts[0].bone < max_bone / 2 {
            self.brain = self.brain.saturating_sub(impact.saturating_mul(2));
            consequence.push_str(" The brain is damaged.");
            serious = true;
        }
        if index == 1 && exposed && organ_roll < 40 {
            if organ_roll < 10 {
                self.heart = self.heart.saturating_sub(impact.saturating_mul(2));
                consequence.push_str(" The heart is injured.");
            } else {
                let lung = (organ_roll % 2) as usize;
                self.lungs[lung] = self.lungs[lung].saturating_sub(impact.saturating_mul(2));
                consequence.push_str(" A lung is damaged; breath comes harder.");
            }
            serious = true;
        }
        self.stamina = self.stamina.min(self.breath_capacity());
        InjuryReport {
            message: format!(
                "The {} takes the blow.{consequence}",
                self.parts[index].kind.name()
            ),
            serious,
        }
    }

    /// Exactly one line per treatment index. Details remain attached to their
    /// region rather than inserting rows that would move the treatment target.
    pub fn condition_lines(&self) -> Vec<String> {
        let (flesh, bone) = Self::durability(self.kind);
        self.parts
            .iter()
            .enumerate()
            .map(|(index, p)| {
                let mut details = Vec::new();
                if p.severed {
                    details.push("severed; permanent loss".to_owned());
                } else {
                    if p.flesh < flesh {
                        details.push(format!("flesh {}/{}", p.flesh, flesh));
                    }
                    if p.bone < bone * 3 / 4 {
                        details.push(
                            if p.splinted {
                                "fracture, splinted"
                            } else {
                                "fracture, unsupported"
                            }
                            .to_owned(),
                        );
                    } else if p.bone < bone {
                        details.push("bone damaged".to_owned());
                    }
                    if p.nerve < 100 {
                        details.push(format!("nerve {}%", p.nerve));
                    }
                }
                if p.bleeding > 0 {
                    details.push(format!("bleeding {}/turn", p.bleeding));
                }
                if index == 0 {
                    if self.eyes != [100, 100] {
                        details.push(format!(
                            "eyes {}/{}%; sight {}",
                            self.eyes[0],
                            self.eyes[1],
                            self.vision_radius()
                        ));
                    }
                    if self.brain < 100 {
                        details.push(format!("brain {}%", self.brain));
                    }
                }
                if index == 1 {
                    if self.heart < 100 {
                        details.push(format!("heart {}%", self.heart));
                    }
                    if self.lungs != [100, 100] {
                        details.push(format!(
                            "lungs {}/{}%; breath cap {}",
                            self.lungs[0],
                            self.lungs[1],
                            self.breath_capacity()
                        ));
                    }
                }
                if (2..=5).contains(&index) && self.part_function(index) < 75 {
                    details.push("weak grip".into());
                }
                if index >= 6 && self.part_function(index) < 75 {
                    details.push("impaired movement".into());
                }
                if details.is_empty() {
                    details.push("sound".to_owned());
                }
                format!("{}: {}", p.kind.name(), details.join(", "))
            })
            .collect()
    }

    /// Reject inconsistent persisted anatomy or equipment before use.
    pub fn validate(&self) -> Result<(), String> {
        let (flesh, bone) = Self::durability(self.kind);
        if self.blood > 1000
            || self.stamina > self.breath_capacity()
            || self.hunger > 100
            || self.nutrition_clock >= 20
            || self.brain > 100
            || self.heart > 100
            || self.eyes.iter().chain(self.lungs.iter()).any(|v| *v > 100)
        {
            return Err("invalid physiology reserves".into());
        }
        for (index, p) in self.parts.iter().enumerate() {
            if p.kind != PartKind::ALL[index]
                || p.flesh > flesh
                || p.bone > bone
                || p.nerve > 100
                || p.bleeding > 100
                || (p.severed && (p.flesh != 0 || p.bone != 0 || p.nerve != 0 || p.splinted))
                || (index == 1 && p.severed)
            {
                return Err("invalid anatomical tissue".into());
            }
            if p.severed
                && p.kind
                    .child()
                    .is_some_and(|child| !self.parts[child].severed)
            {
                return Err("detached limb retained its extremity".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Finite consumables carried by the explorer.
pub struct Supplies {
    /// Remaining individual linen dressings.
    pub bandages: u16,
    /// Remaining supports for fractured limbs.
    pub splints: u16,
    /// Remaining rations, each restoring up to fifty nutrition.
    pub food: u16,
}

impl Default for Supplies {
    fn default() -> Self {
        Self {
            bandages: 4,
            splints: 2,
            food: 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Whether a care action changed physiology or consumed supplies, with narration.
pub struct CareResult {
    /// Whether this action changed the body or its available supplies.
    pub changed: bool,
    /// Narration suitable for the expedition journal.
    pub message: String,
}

impl CareResult {
    fn changed(message: impl Into<String>) -> Self {
        Self {
            changed: true,
            message: message.into(),
        }
    }
    fn unchanged(message: impl Into<String>) -> Self {
        Self {
            changed: false,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Compact weapon catalogue with distinct damage and timing tradeoffs.
pub enum WeaponKind {
    /// Weak natural or unarmed impact; available after weapon use becomes impossible.
    Unarmed,
    /// Quick cutting weapon that opens bleeding wounds.
    Knife,
    /// Two-handed reach weapon that penetrates deeper tissue.
    Spear,
    /// Slow heavy weapon that crushes bone through armor.
    Mace,
}

impl WeaponKind {
    /// Short player-facing name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Unarmed => "bare hands",
            Self::Knife => "knife",
            Self::Spear => "spear",
            Self::Mace => "mace",
        }
    }
    /// Maximum attack range in tiles.
    pub fn reach(self) -> i32 {
        if self == Self::Spear { 2 } else { 1 }
    }
    /// Base attack duration in simulation time units.
    pub fn cost(self) -> u64 {
        match self {
            Self::Unarmed | Self::Knife => 100,
            Self::Spear => 150,
            Self::Mace => 200,
        }
    }
    /// Breath spent by one attack.
    pub fn breath_cost(self) -> u16 {
        match self {
            Self::Unarmed => 4,
            Self::Knife => 6,
            Self::Spear => 10,
            Self::Mace => 15,
        }
    }
    /// Nominal impact strength for this weapon.
    pub fn power(self) -> u16 {
        match self {
            Self::Unarmed => 8,
            Self::Knife => 22,
            Self::Spear => 28,
            Self::Mace => 36,
        }
    }
    /// Carried weight used to determine movement and sprint exertion.
    pub fn weight(self) -> u16 {
        match self {
            Self::Unarmed => 0,
            Self::Knife => 1,
            Self::Spear => 3,
            Self::Mace => 5,
        }
    }
    /// Describe protection or attack tradeoffs for equipment comparison.
    pub fn description(self) -> String {
        format!(
            "{}: reach {}, {} time, {} breath; {}",
            self.name(),
            self.reach(),
            self.cost(),
            self.breath_cost(),
            match self {
                Self::Unarmed => "weak blunt blows",
                Self::Knife => "quick bleeding cuts",
                Self::Spear => "pierces deep tissue; 200 time adjacent; requires two hands",
                Self::Mace => "crushes bone through armor",
            }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Regions that can hold one piece of protective equipment each.
pub enum ArmorSlot {
    /// Head, including eyes and the protected brain.
    Head,
    /// Chest and abdomen, including heart and lungs.
    Torso,
    /// Upper limbs between shoulders and gripping extremities.
    Arms,
    /// Gripping extremities.
    Hands,
    /// Lower limbs between hips and feet.
    Legs,
    /// Weight-bearing extremities.
    Feet,
}

impl ArmorSlot {
    /// All variants in their stable storage and inspection order.
    pub const ALL: [Self; 6] = [
        Self::Head,
        Self::Torso,
        Self::Arms,
        Self::Hands,
        Self::Legs,
        Self::Feet,
    ];
    /// Stable index into the equipment armor array.
    pub fn index(self) -> usize {
        self as usize
    }
    /// Short player-facing name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Torso => "torso",
            Self::Arms => "arms",
            Self::Hands => "hands",
            Self::Legs => "legs",
            Self::Feet => "feet",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Protective materials, ordered from light clothing to heavy metal.
pub enum ArmorMaterial {
    /// Light fabric with minimal protection.
    Cloth,
    /// Moderate weight and useful resistance to cuts.
    Leather,
    /// Heavy protection against cuts with greater vulnerability to crushing.
    Iron,
}

impl ArmorMaterial {
    /// Short player-facing name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Cloth => "cloth",
            Self::Leather => "leather",
            Self::Iron => "iron",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Protection for a region, including material and coverage tradeoffs.
pub struct ArmorPiece {
    /// The body region covered by this armor piece.
    pub slot: ArmorSlot,
    /// Material determining protection and carried weight.
    pub material: ArmorMaterial,
}

impl ArmorPiece {
    /// Short player-facing name.
    pub fn name(self) -> String {
        format!("{} {} armor", self.material.name(), self.slot.name())
    }
    /// Percentage chance that this piece intercepts an impact to its region.
    pub fn coverage(self) -> u16 {
        match (self.slot, self.material) {
            (ArmorSlot::Head, _) => 65, // open face remains a tactical vulnerability
            (_, ArmorMaterial::Cloth) => 75,
            (_, ArmorMaterial::Leather) => 85,
            (_, ArmorMaterial::Iron) => 95,
        }
    }
    /// Impact absorbed when this material intercepts the given weapon.
    pub fn protection(self, weapon: WeaponKind) -> u16 {
        match (self.material, weapon) {
            (ArmorMaterial::Cloth, _) => 2,
            (ArmorMaterial::Leather, WeaponKind::Knife) => 10,
            (ArmorMaterial::Leather, _) => 5,
            (ArmorMaterial::Iron, WeaponKind::Knife) => 24,
            (ArmorMaterial::Iron, WeaponKind::Spear) => 14,
            (ArmorMaterial::Iron, _) => 8,
        }
    }
    /// Carried weight used to determine movement and sprint exertion.
    pub fn weight(self) -> u16 {
        let base = match self.material {
            ArmorMaterial::Cloth => 1,
            ArmorMaterial::Leather => 3,
            ArmorMaterial::Iron => 6,
        };
        base * if self.slot == ArmorSlot::Torso { 2 } else { 1 }
    }
    /// Describe protection or attack tradeoffs for equipment comparison.
    pub fn description(self) -> String {
        format!(
            "{}: {}% coverage, weight {}; cut/pierce/blunt {}/{}/{}",
            self.name(),
            self.coverage(),
            self.weight(),
            self.protection(WeaponKind::Knife),
            self.protection(WeaponKind::Spear),
            self.protection(WeaponKind::Mace)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Active weapon, optional spare, and independent armor regions.
pub struct Equipment {
    /// Readied weapon; injuries may force an unarmed fallback.
    pub active: WeaponKind,
    /// One carried spare weapon, if any.
    pub spare: Option<WeaponKind>,
    /// At most one piece per slot, indexed by `ArmorSlot::index`.
    pub armor: [Option<ArmorPiece>; 6],
}

impl Default for Equipment {
    fn default() -> Self {
        let mut armor = [None; 6];
        armor[ArmorSlot::Torso.index()] = Some(ArmorPiece {
            slot: ArmorSlot::Torso,
            material: ArmorMaterial::Cloth,
        });
        Self {
            active: WeaponKind::Knife,
            spare: None,
            armor,
        }
    }
}

impl Equipment {
    /// Carried weight used to determine movement and sprint exertion.
    pub fn weight(&self) -> u16 {
        self.active.weight()
            + self.spare.map_or(0, WeaponKind::weight)
            + self.armor.iter().flatten().map(|a| a.weight()).sum::<u16>()
    }
    /// Equipment comparison lines including carried weight and exposed regions.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Held: {}", self.active.description()),
            format!(
                "Spare: {}",
                self.spare
                    .map_or_else(|| "none".into(), WeaponKind::description)
            ),
        ];
        for slot in ArmorSlot::ALL {
            lines.push(self.armor[slot.index()].map_or_else(
                || format!("{}: exposed", slot.name()),
                ArmorPiece::description,
            ));
        }
        lines.push(format!("Total carried weight: {}", self.weight()));
        lines
    }
    /// Reject inconsistent persisted anatomy or equipment before use.
    pub fn validate(&self) -> Result<(), String> {
        if self.spare == Some(WeaponKind::Unarmed) {
            return Err("bare hands cannot be carried as a spare".into());
        }
        if self
            .armor
            .iter()
            .enumerate()
            .any(|(index, armor)| armor.is_some_and(|a| a.slot.index() != index))
        {
            return Err("armor equipped on the wrong region".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Raw impact delivered to the target anatomy before armor mitigation.
pub struct AttackProfile {
    /// Weapon shape determining tissue penetration and wound type.
    pub weapon: WeaponKind,
    /// Raw impact strength before target armor and tissue layers.
    pub power: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Player-readable consequences of a resolved physical impact.
pub struct InjuryReport {
    /// Narration suitable for the expedition journal.
    pub message: String,
    /// Whether this impact caused a major wound or structural injury.
    pub serious: bool,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use proptest::prelude::*;

    fn unarmored() -> Equipment {
        Equipment {
            active: WeaponKind::Unarmed,
            spare: None,
            armor: [None; 6],
        }
    }

    fn structural_state(body: &Body) -> Vec<u16> {
        let mut tissue = body
            .parts
            .iter()
            .flat_map(|p| [p.bone, p.nerve, u16::from(p.severed)])
            .collect::<Vec<_>>();
        tissue.extend(body.eyes);
        tissue.extend(body.lungs);
        tissue.extend([body.heart, body.brain]);
        tissue
    }

    fn survivor_with(predicate: impl Fn(&Body) -> bool) -> Body {
        for seed in 0..10_000 {
            let mut body = Body::default();
            body.hit(
                AttackProfile {
                    weapon: WeaponKind::Spear,
                    power: 180,
                },
                &unarmored(),
                &mut Rng(seed),
            );
            if !body.is_dead() && predicate(&body) {
                return body;
            }
        }
        panic!("no matching injury in deterministic scenario seeds")
    }

    #[test]
    fn walking_time_never_recovers_breath_and_healthy_sprinting_is_faster() {
        let mut body = Body::default();
        let gear = Equipment::default();
        body.stamina = 40;
        for _ in 0..100 {
            body.tick();
        }
        assert_eq!(body.stamina, 40);
        assert_eq!(body.movement_cost(false, &gear), 100);
        assert_eq!(body.movement_cost(true, &gear), 50);
        body.wait();
        assert!(body.stamina > 40);
    }

    #[test]
    fn automatic_care_stabilizes_an_actual_severed_limb_without_regrowing_it() {
        let mut body = survivor_with(|b| b.parts[6].severed);
        let lasting = structural_state(&body);
        let mut supplies = Supplies::default();
        assert!(body.bleeding() > 0);
        assert!(body.parts[8].severed, "a lost leg includes its foot");
        let result = body.care_step(&mut supplies);
        assert!(result.changed);
        assert_eq!(supplies.bandages, 3);
        assert_eq!(body.bleeding(), 0);
        let mut steps = 0;
        while body.can_recover(&supplies) && steps < 400 {
            body.care_step(&mut supplies);
            body.tick();
            steps += 1;
        }
        assert!(
            steps < 400,
            "automatic care must finish with permanent injuries"
        );
        assert_eq!(structural_state(&body), lasting);
        assert!(body.movement_cost(false, &Equipment::default()) > 100);
        assert!(body.validate().is_ok());
        body.restore();
        assert_eq!(body, Body::default());
    }

    #[test]
    fn food_and_splints_follow_urgent_bleeding() {
        let mut injured = None;
        for seed in 0..1000 {
            let mut body = Body::default();
            body.hit(
                AttackProfile {
                    weapon: WeaponKind::Mace,
                    power: 45,
                },
                &unarmored(),
                &mut Rng(seed),
            );
            if body.splint_target().is_some() && body.bleeding() > 0 {
                injured = Some(body);
                break;
            }
        }
        let mut body = injured.expect("crushing limb injury");
        body.hunger = 40;
        let mut supplies = Supplies::default();
        let bones = structural_state(&body);
        body.care_step(&mut supplies);
        assert_eq!(
            (supplies.bandages, supplies.splints, supplies.food),
            (3, 2, 3)
        );
        body.care_step(&mut supplies);
        assert_eq!(
            (supplies.bandages, supplies.splints, supplies.food),
            (3, 1, 3)
        );
        assert!(body.parts.iter().any(|p| p.splinted));
        body.care_step(&mut supplies);
        assert_eq!(supplies.food, 2);
        assert_eq!(body.hunger, 90);
        assert_eq!(structural_state(&body), bones);
    }

    #[test]
    fn selected_treatment_binds_then_splints_only_the_selected_limb() {
        let mut candidate = None;
        for seed in 0..1000 {
            let mut body = Body::default();
            let mut rng = Rng(seed);
            for _ in 0..2 {
                body.hit(
                    AttackProfile {
                        weapon: WeaponKind::Mace,
                        power: 45,
                    },
                    &unarmored(),
                    &mut rng,
                );
            }
            if !body.is_dead()
                && body
                    .parts
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| body.needs_splint(*i))
                    .count()
                    >= 2
            {
                candidate = Some(body);
                break;
            }
        }
        let mut body = candidate.expect("two independently injured limbs");
        let target = body.splint_target().unwrap();
        let other_parts: Vec<_> = body
            .parts
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != target)
            .map(|(_, part)| part.clone())
            .collect();
        let mut supplies = Supplies::default();
        assert!(body.treat(&mut supplies, target).changed);
        assert_eq!(supplies.bandages, 3);
        assert!(!body.parts[target].splinted);
        assert!(body.treat(&mut supplies, target).changed);
        assert_eq!(supplies.splints, 1);
        assert!(body.parts[target].splinted);
        assert_eq!(
            body.parts
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != target)
                .map(|(_, part)| part.clone())
                .collect::<Vec<_>>(),
            other_parts
        );
        let remaining = body.splint_target().unwrap();
        supplies.bandages = 0;
        assert!(body.treat(&mut supplies, remaining).changed);
        assert!(body.parts[remaining].splinted);
        assert!(
            body.parts[remaining].bleeding > 0,
            "splints never stop bleeding"
        );
        let unchanged = (body.clone(), supplies.clone());
        assert!(!body.treat(&mut supplies, usize::MAX).changed);
        assert_eq!((body, supplies), unchanged);
    }

    #[test]
    fn exhausted_supplies_do_not_create_an_endless_recovery_loop() {
        let mut body = survivor_with(|b| b.bleeding() > 0);
        body.stamina = 0;
        let mut supplies = Supplies {
            bandages: 0,
            splints: 0,
            food: 0,
        };
        for _ in 0..100 {
            if !body.can_recover(&supplies) {
                break;
            }
            body.care_step(&mut supplies);
            body.tick();
        }
        assert!(!body.can_recover(&supplies));
        assert!(body.bleeding() > 0);
    }

    #[test]
    fn bilateral_eye_loss_reduces_current_sight_to_one_tile() {
        let mut body = Body {
            eyes: [0, 100],
            ..Body::default()
        };
        assert_eq!(body.vision_radius(), 5);
        body.eyes = [0, 0];
        assert_eq!(body.vision_radius(), 1);
        let mut supplies = Supplies::default();
        body.care_step(&mut supplies);
        assert_eq!(body.eyes, [0, 0]);
        body.restore();
        assert_eq!(body.vision_radius(), 7);
        assert_eq!(Body::new(BodyKind::Brute).vision_radius(), 2);
    }

    #[test]
    fn armor_protects_against_cuts_but_retains_weight_and_blunt_weaknesses() {
        let iron = Equipment {
            active: WeaponKind::Knife,
            spare: None,
            armor: ArmorSlot::ALL.map(|slot| {
                Some(ArmorPiece {
                    slot,
                    material: ArmorMaterial::Iron,
                })
            }),
        };
        let mut naked_loss = 0;
        let mut iron_loss = 0;
        let mut blunt_loss = 0;
        for seed in 0..500 {
            for (gear, weapon, total) in [
                (&unarmored(), WeaponKind::Knife, &mut naked_loss),
                (&iron, WeaponKind::Knife, &mut iron_loss),
                (&iron, WeaponKind::Mace, &mut blunt_loss),
            ] {
                let mut body = Body::default();
                body.hit(AttackProfile { weapon, power: 24 }, gear, &mut Rng(seed));
                *total += 1000_u32 - u32::from(body.blood);
            }
        }
        assert!(iron_loss < naked_loss / 2);
        assert!(blunt_loss > iron_loss);
        let body = Body::default();
        assert!(body.sprint_cost(&iron) > body.sprint_cost(&unarmored()));
        assert!(body.movement_cost(false, &iron) > body.movement_cost(false, &unarmored()));
    }

    #[test]
    fn rats_are_fragile_and_vital_tissue_damage_is_fatal_for_every_species() {
        let hit = AttackProfile {
            weapon: WeaponKind::Knife,
            power: 25,
        };
        for seed in 0..100 {
            let mut rat = Body::new(BodyKind::Rat);
            let mut human = Body::default();
            for _ in 0..3 {
                rat.hit(hit, &unarmored(), &mut Rng(seed));
                human.hit(hit, &unarmored(), &mut Rng(seed));
            }
            assert!(rat.is_dead());
            assert!(rat.blood <= human.blood);
        }
        for kind in [BodyKind::Human, BodyKind::Rat, BodyKind::Brute] {
            let mut body = Body::new(kind);
            body.heart = 0;
            assert!(body.is_dead());
            let before = body.clone();
            body.care_step(&mut Supplies::default());
            body.tick();
            body.wait();
            assert_eq!(body, before, "ordinary actions cannot revive the dead");
        }
    }

    proptest! {
        #[test]
        fn anatomy_remains_valid_and_care_never_repairs_structural_damage(
            seed in any::<u64>(),
            kind in 0_u8..3,
            actions in prop::collection::vec((0_u8..7, any::<u16>()), 1..150),
        ) {
            let kind = [BodyKind::Human, BodyKind::Rat, BodyKind::Brute][kind as usize];
            let mut body = Body::new(kind);
            let mut rng = Rng(seed);
            let mut supplies = Supplies::default();
            for (action, magnitude) in actions {
                match action {
                    0..=2 => {
                        let weapon = [WeaponKind::Knife, WeaponKind::Spear, WeaponKind::Mace][action as usize];
                        body.hit(AttackProfile { weapon, power: magnitude }, &Equipment::default(), &mut rng);
                    }
                    3 => body.tick(),
                    4 => body.wait(),
                    5 => {
                        let structural = structural_state(&body);
                        body.care_step(&mut supplies);
                        prop_assert_eq!(structural_state(&body), structural);
                    }
                    _ => body.restore(),
                }
                prop_assert!(body.validate().is_ok(), "{:?}", body.validate());
                let encoded = serde_json::to_vec(&body).unwrap();
                let decoded: Body = serde_json::from_slice(&encoded).unwrap();
                prop_assert_eq!(&body, &decoded);
                prop_assert!(body.movement_cost(false, &Equipment::default()) > 0);
                prop_assert!(body.attack_power(22) > 0);
            }
        }
    }
}
