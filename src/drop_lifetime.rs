//! Opt-in, bounded USA drop lifetimes. Collection stays in guest game code.
use gba_core::Machine;
use std::collections::HashMap;
const CASH: usize = 0x35cf0;
const STRIDE: usize = 0x68;
const REMOVE: u32 = 0x400;
fn word(b: &[u8], a: u32) -> Option<u32> {
    let p = a.checked_sub(0x02000000)? as usize;
    Some(u32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    const ENTITY: u32 = 0x02028000;
    const TIMER: u32 = 0x0203020c;
    fn fixture() -> Machine {
        let mut rom = vec![0; 0x200];
        rom[0xac..0xb0].copy_from_slice(b"BGTE");
        let mut m = Machine::new(rom);
        for (a, v) in [
            (0x02022f50, 0x02022e50),
            (0x02022e58, 0x02022e60),
            (0x02022e40, 0x02022ac0),
            (0x02022ad4, 0x02022adc),
        ] {
            put(&mut m.bus.ewram, a, v);
        }
        for (p, v) in [
            (0x110c2, 1u16),
            (0x110c4, 5),
            (0x110cc, 64),
            (0x110d0, 0x7777),
        ] {
            m.bus.ewram[p..p + 2].copy_from_slice(&v.to_le_bytes());
        }
        m
    }
    fn corpse(m: &mut Machine, flags: u32) {
        for (a, v) in [
            (ENTITY + 8, 8),
            (ENTITY + 12, flags),
            (ENTITY + 24, 0x02004000),
            (ENTITY + 40, 0x02030000),
            (0x02030024, TIMER - 12),
        ] {
            put(&mut m.bus.ewram, a, v);
        }
        m.bus.ewram[(TIMER - 0x02000000 - 2) as usize] = 1;
        m.bus.ewram[(TIMER - 0x02000000 - 4) as usize] = 2;
        m.cpu.regs[5] = ENTITY;
        m.cpu.regs[1] = TIMER;
        m.cpu.regs[0] = 120;
    }
    fn weapon(m: &mut Machine, policy: &mut Policy, e: u32) {
        for (a, v) in [(e + 8, 0x5d80), (e + 12, 0x10105001), (e + 24, e + 0x4000)] {
            put(&mut m.bus.ewram, a, v);
        }
        m.cpu.regs[4] = e;
        policy.observe(m, 0x080609f1);
    }
    #[test]
    fn thirty_seconds_and_original_cash_value_decay() {
        let mut m = fixture();
        let mut p = Policy::new(30);
        assert_eq!(p.frames, 1792);
        m.cpu.regs[12] = 0x02035cf0;
        p.observe(&mut m, 0x08061b5b);
        m.cpu.regs[7] = 0x02035d54;
        m.cpu.regs[8] = 0;
        m.bus.ewram[CASH + 0x5a] = 22;
        m.cpu.regs[1] = 22;
        m.bus.frames = 1791;
        p.observe(&mut m, 0x0806069b);
        assert_eq!(m.cpu.regs[1], 21);
        assert_eq!(m.bus.ewram[CASH + 0x5a], 21);
        m.bus.frames = 1792;
        p.observe(&mut m, 0x0806069b);
        assert_eq!(m.cpu.regs[1], 22);
        m.bus.frames = 5;
        m.bus.ewram[CASH + 0x5b] = 1;
        p.observe(&mut m, 0x0806069b);
        assert_eq!(m.cpu.regs[1], 22);
    }
    #[test]
    fn corpse_deadline_and_actor_reuse() {
        let mut m = fixture();
        let mut p = Policy::new(30);
        // Both ordinary deaths and deaths with bit 29 must be recognized.
        for flags in [0x00115001, 0x20911001] {
            corpse(&mut m, flags);
            p.observe(&mut m, 0x0804976d);
            assert_eq!(m.cpu.regs[0], 120);
            p.update(&mut m);
            assert_eq!(p.bodies.len(), 1);
        }
        m.bus.frames = 1792;
        p.update(&mut m);
        assert_ne!(word(&m.bus.ewram, ENTITY + 12).unwrap() & REMOVE, 0);
        p.update(&mut m);
        assert!(p.bodies.is_empty());
        m.bus.frames = 0;
        corpse(&mut m, 0x00115001);
        p.observe(&mut m, 0x0804976d);
        put(&mut m.bus.ewram, ENTITY + 24, 0x02005000);
        let before = m.bus.ewram.clone();
        p.update(&mut m);
        assert!(p.bodies.is_empty());
        assert_eq!(m.bus.ewram, before);
        corpse(&mut m, 0x00115001);
        p.observe(&mut m, 0x0804976d);
        // Same pointers and substate, but another AI action: never hold it.
        m.bus.ewram[(TIMER - 0x02000000 - 4) as usize] = 20;
        let before = m.bus.ewram.clone();
        p.update(&mut m);
        assert!(p.bodies.is_empty());
        assert_eq!(m.bus.ewram, before);
    }
    #[test]
    fn excludes_player_mission_and_changed_state() {
        for flags in [0x00115011, 0x00515001] {
            let mut m = fixture();
            corpse(&mut m, flags);
            let mut p = Policy::new(30);
            p.observe(&mut m, 0x0804976d);
            assert!(p.bodies.is_empty());
        }
        let mut m = fixture();
        corpse(&mut m, 0x00115001);
        put(&mut m.bus.ewram, 0x02000f10, ENTITY);
        let mut p = Policy::new(30);
        p.observe(&mut m, 0x0804976d);
        assert!(p.bodies.is_empty());
        put(&mut m.bus.ewram, 0x02000f10, 0);
        p.observe(&mut m, 0x0804976d);
        m.bus.ewram[(TIMER - 0x02000000 - 2) as usize] = 0;
        let before = m.bus.ewram.clone();
        p.update(&mut m);
        assert!(p.bodies.is_empty());
        assert_eq!(m.bus.ewram, before);
    }
    #[test]
    fn weapon_budget_expiry_and_native_pickup_removal() {
        let mut m = fixture();
        let mut p = Policy::new(30);
        for i in 0..9 {
            weapon(&mut m, &mut p, ENTITY + i * 0x80);
        }
        assert_ne!(word(&m.bus.ewram, ENTITY + 12).unwrap() & REMOVE, 0);
        p.update(&mut m);
        assert_eq!(p.weapons.len(), 8);
        let e = ENTITY + 0x80;
        assert!(p.weapons[&e].protected);
        assert_eq!(word(&m.bus.ewram, e + 12).unwrap() & 0x10, 0);
        let before = m.bus.ewram.clone();
        m.cpu.regs[2] = e;
        m.cpu.regs[1] = word(&m.bus.ewram, e + 12).unwrap() | REMOVE;
        p.observe(&mut m, 0x08061da3);
        assert_eq!(m.cpu.regs[1], word(&m.bus.ewram, e + 12).unwrap());
        assert_eq!(m.bus.ewram, before);
        m.cpu.regs[4] = e;
        m.cpu.regs[0] = REMOVE;
        p.observe(&mut m, 0x08060849);
        assert_eq!(m.cpu.regs[0], 0);
        // Native dynamic-drop flag remains set: this does not make a respawning pickup.
        assert_ne!(word(&m.bus.ewram, e + 12).unwrap() & 0x4000, 0);
        put(&mut m.bus.ewram, e + 12, 0x10105411);
        p.update(&mut m);
        assert!(!p.weapons.contains_key(&e));
        m.bus.frames = p.frames;
        p.update(&mut m);
        assert!(p
            .weapons
            .keys()
            .all(|e| word(&m.bus.ewram, *e + 12).unwrap() & REMOVE != 0));
    }
    #[test]
    fn pressure_retires_one_body_then_allows_original_cleanup_if_reserves_fail() {
        let mut m = fixture();
        let mut p = Policy::new(30);
        corpse(&mut m, 0x00115001);
        p.observe(&mut m, 0x0804976d);
        weapon(&mut m, &mut p, ENTITY + 0x80);
        put(&mut m.bus.ewram, 0x02022e40, 0);
        p.update(&mut m);
        p.update(&mut m);
        assert_eq!(p.emergency_bodies, 1);
        let e = ENTITY + 0x80;
        assert!(p.weapons[&e].protected);
        put(&mut m.bus.ewram, 0x02022f50, 0);
        p.update(&mut m);
        assert!(!p.weapons[&e].protected);
    }
}
fn put(b: &mut [u8], a: u32, v: u32) {
    if let Some(p) = a
        .checked_sub(0x02000000)
        .and_then(|p| b.get_mut(p as usize..p as usize + 4))
    {
        p.copy_from_slice(&v.to_le_bytes());
    }
}
#[derive(Clone, Copy)]
struct Entry {
    born: u64,
    position: u32,
    identity: u32,
    timer: u32,
    action: u16,
    protected: bool,
}
pub struct Policy {
    frames: u64,
    cash: [Option<u64>; 4],
    bodies: HashMap<u32, Entry>,
    weapons: HashMap<u32, Entry>,
    recovery: Option<(u32, Entry)>,
    pub held: u64,
    pub emergency_bodies: u64,
    pub expired: u64,
}
impl Policy {
    pub fn new(seconds: u32) -> Self {
        Self {
            frames: (seconds as u64 * 16_777_216).div_ceil(280_896),
            cash: [None; 4],
            bodies: HashMap::new(),
            weapons: HashMap::new(),
            recovery: None,
            held: 0,
            emergency_bodies: 0,
            expired: 0,
        }
    }
    #[inline]
    pub fn interested(key: u32) -> bool {
        matches!(
            key,
            0x08061b5b
                | 0x0806069b
                | 0x0804976d
                | 0x080609f1
                | 0x08060849
                | 0x08061be7
                | 0x08061da3
        )
    }
    fn entry(m: &Machine, e: u32, timer: u32) -> Option<Entry> {
        Some(Entry {
            born: m.bus.frames,
            position: word(&m.bus.ewram, e + 24)?,
            identity: word(&m.bus.ewram, e + 8)?,
            timer,
            action: timer
                .checked_sub(4)
                .and_then(|a| word(&m.bus.ewram, a))
                .unwrap_or(0) as u16,
            protected: false,
        })
    }
    fn valid(m: &Machine, e: u32, s: Entry) -> bool {
        word(&m.bus.ewram, e + 24) == Some(s.position)
            && word(&m.bus.ewram, e + 8) == Some(s.identity)
            && word(&m.bus.ewram, e + 12).is_some_and(|f| f != 0xfdfdfdfd && f & REMOVE == 0)
    }
    fn corpse_timer(m: &Machine, e: u32) -> Option<u32> {
        let a = word(&m.bus.ewram, e + 40)?;
        let b = word(&m.bus.ewram, a + 36)?;
        b.checked_add(12)
    }
    pub fn observe(&mut self, m: &mut Machine, key: u32) {
        if m.bus.rom.get(0xac..0xb0) != Some(b"BGTE") {
            return;
        }
        match key {
            0x08061b5b => {
                if let Some(o) = m.cpu.regs[12].checked_sub(0x02035cf0) {
                    if o % STRIDE as u32 == 0 && o / (STRIDE as u32) < 4 {
                        self.cash[(o / STRIDE as u32) as usize] = Some(m.bus.frames);
                    }
                }
            }
            0x0806069b => {
                let o = m.cpu.regs[8] as usize;
                if o % STRIDE == 0
                    && o / STRIDE < 4
                    && m.cpu.regs[7] == 0x02035d54 + o as u32
                    && m.bus.ewram[CASH + o + 0x5b] == 0
                {
                    if let Some(born) = self.cash[o / STRIDE] {
                        if m.bus.frames.saturating_sub(born) < self.frames {
                            if m.cpu.regs[1] > 21 {
                                m.cpu.regs[1] = 21;
                                m.bus.ewram[CASH + o + 0x5a] = 21;
                                self.held += 1;
                            }
                        } else {
                            m.cpu.regs[1] = 22;
                        }
                    }
                }
            }
            0x0804976d => {
                let e = m.cpu.regs[5];
                let timer = m.cpu.regs[1];
                let b = &m.bus.ewram;
                let flags = word(b, e + 12).unwrap_or(0);
                let kind = word(b, e + 8).unwrap_or(0) & 255;
                if matches!(kind, 4 | 8)
                    && flags & 0x00400010 == 0
                    && word(b, 0x02000f10) != Some(e)
                    && Self::corpse_timer(m, e) == Some(timer)
                {
                    if let Some(s) = Self::entry(m, e, timer) {
                        self.bodies.insert(e, s);
                    }
                }
            }
            0x080609f1 => {
                let e = m.cpu.regs[4];
                let b = &m.bus.ewram;
                let flags = word(b, e + 12).unwrap_or(0);
                if word(b, e + 8).unwrap_or(0) & 255 == 128 && flags & 0x10004000 == 0x10004000 {
                    if let Some(s) = Self::entry(m, e, 0) {
                        self.weapons.insert(e, s);
                    }
                    self.update(m);
                }
            }
            0x08060849 => {
                let e = m.cpu.regs[4];
                if self.weapons.get(&e).is_some_and(|s| {
                    s.protected
                        && Self::valid(m, e, *s)
                        && m.bus.frames.saturating_sub(s.born) < self.frames
                }) {
                    m.cpu.regs[0] = 0;
                }
            }
            0x08061da3 => {
                let e = m.cpu.regs[2];
                if self.weapons.get(&e).is_some_and(|s| {
                    s.protected
                        && Self::valid(m, e, *s)
                        && m.bus.frames.saturating_sub(s.born) < self.frames
                }) && word(&m.bus.ewram, e + 12) == Some(m.cpu.regs[1] & !REMOVE)
                {
                    // Suppress just this pressure deletion, leaving native
                    // pickup, respawn and culling flags entirely untouched.
                    m.cpu.regs[1] &= !REMOVE;
                    self.held += 1;
                }
            }
            0x08061be7 if m.cpu.regs[0] != 0 => self.update(m),
            _ => {}
        }
    }
    pub fn update(&mut self, m: &mut Machine) {
        self.bodies.retain(|e, s| {
            Self::valid(m, *e, *s)
                && Self::corpse_timer(m, *e) == Some(s.timer)
                && s.timer.checked_sub(4).and_then(|a| word(&m.bus.ewram, a))
                    .is_some_and(|v| v as u16 == s.action)
                // State 1 is the corpse countdown in 080496a4. A particular
                // entity flag is insufficient: different death types set it
                // differently. Stop if the actor changes state or is reused.
                && s.timer.checked_sub(2).is_some_and(|a| {
                    a.checked_sub(0x02000000)
                        .and_then(|p| m.bus.ewram.get(p as usize..p as usize + 2))
                        == Some(&[1, 0][..])
                })
                && word(&m.bus.ewram, *e + 12)
                    .is_some_and(|f| f & 0x00400010 == 0)
                && word(&m.bus.ewram, 0x02000f10) != Some(*e)
        });
        self.weapons.retain(|e, s| {
            Self::valid(m, *e, *s)
                && word(&m.bus.ewram, *e + 12).unwrap_or(0) & 0x10004000 == 0x10004000
        });
        let now = m.bus.frames;
        for (&e, s) in &self.bodies {
            if now.saturating_sub(s.born) >= self.frames {
                let f = word(&m.bus.ewram, e + 12).unwrap();
                put(&mut m.bus.ewram, e + 12, f | REMOVE);
                self.expired += 1;
            } else if let Some(p) = s.timer.checked_sub(0x02000000).map(|p| p as usize) {
                if let Some(v) = m.bus.ewram.get_mut(p..p + 2) {
                    // Hold the game's original countdown. Host frame time is
                    // authoritative; do not assume a fixed number of guest
                    // logic updates per display frame or allow an early expiry.
                    v.copy_from_slice(&120u16.to_le_bytes());
                    self.held += 1;
                }
            }
        }
        // Preserve a resource escape hatch: retire one oldest dead NPC, never a
        // living/mission/player actor, before protecting weapon drops under pressure.
        let spare = crate::cash_protection::spare_resources(&m.bus.ewram);
        let matrices = crate::pickup_probe::affine_free(&m.bus.ewram).unwrap_or(0);
        self.recovery = self.recovery.filter(|(e, s)| {
            word(&m.bus.ewram, *e + 24) == Some(s.position)
                && word(&m.bus.ewram, *e + 8) == Some(s.identity)
                && word(&m.bus.ewram, *e + 12).is_some_and(|f| f & REMOVE != 0)
        });
        let mut recovering = self.recovery.is_some();
        if spare && matrices <= 1 && !recovering {
            if let Some((&e, _)) = self
                .bodies
                .iter()
                .filter(|(e, s)| Self::valid(m, **e, **s))
                .min_by_key(|(e, s)| (s.born, **e))
            {
                let s = self.bodies[&e];
                let f = word(&m.bus.ewram, e + 12).unwrap();
                put(&mut m.bus.ewram, e + 12, f | REMOVE);
                self.emergency_bodies += 1;
                recovering = true;
                self.recovery = Some((e, s));
            }
        }
        let mut order: Vec<_> = self.weapons.iter().map(|(&e, s)| (s.born, e)).collect();
        order.sort_unstable();
        let excess = order.len().saturating_sub(8);
        for (i, (_, e)) in order.into_iter().enumerate() {
            let s = self.weapons[&e];
            let mut f = word(&m.bus.ewram, e + 12).unwrap();
            if i < excess || now.saturating_sub(s.born) >= self.frames {
                f |= REMOVE;
                self.expired += 1;
            }
            self.weapons.get_mut(&e).unwrap().protected =
                spare && (matrices > 1 || recovering) && f & REMOVE == 0;
            put(&mut m.bus.ewram, e + 12, f);
        }
    }
}
