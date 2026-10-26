# Extended drop lifetime (USA)

**English** | [Русский](../ru/DROP_LIFETIME.md)

Normal `Play.bat` launches use a target lifetime of 30 seconds.
`Play.bat -DropLifetime 0` restores original rules;
`Play.bat -DropLifetime N` selects another duration, from 1 to 300 seconds.
Thirty seconds corresponds to 1792 GBA frames at 16777216/280896 Hz,
independent of the monitor refresh rate.

This changes game behavior independently of Enhanced/Original rendering.
The ROM file and user save are not patched. Session metadata records
`drop_lifetime_seconds`, `drop_lifetime_after_frame` and `preserve_cash`.
Native replays automatically read these values from the adjacent `info.txt`.

## Retention rules

- Money: lifetime starts when a cash pickup is created at `08061b5a`; stage 21
  is held until its deadline at `0806069a`. Original value decay remains:
  the stage is not reset to restore the maximum payout. Collected cash is
  not retained. Affine-resource cleanup protection uses a narrowly scoped
  hook. The original four slots are not expanded, so new cash can replace
  an older pickup.
- Corpses: the initial timer write at `0804976c` is observed. Checks validate
  the actor's data chain, type and absence of player/mission-object flags.
  Only state 1 of the same AI task, timer and object generation is retained,
  using identity and position-pointer checks. The original counter is held
  at 120 while the host counts elapsed frames; logic updates per frame are
  not assumed to be constant. Intervention stops when the task/state changes
  or the memory is reused.
- Weapons: only newly created dynamic type-128 objects with mask `0x10004000`
  qualify. While resources permit, the host marks a weapon as protected in
  its own table. At `08061da2`, only the specific cleanup write of the removal
  flag is suppressed; at `08060848`, eviction of a still-valid protected item
  is suppressed. Persistent object flags, including `0x10` and `0x4000`, are
  not changed. Pickup, respawn and out-of-area removal rules remain original.
  The host retains at most eight weapons, replacing the oldest with a newer
  one. The game still performs collection and awards the pickup.
- Expiration and emergency release use the original removal flag `0x400`.
  The game frees resources; the host does not modify its object lists.

When only 0–1 affine matrices remain, one oldest retained corpse is marked
for removal first. Another is not selected until the game finishes that
removal. If there is no eligible corpse, or palette/heap resources are
insufficient, weapon protection is lifted and original cleanup is allowed.
Thirty seconds is therefore a target, not a guarantee that unlimited objects
can survive exhausted GBA resources. Removed objects are never resurrected.

For diagnostic replays, `--drop-lifetime-after-frame N` postpones activation
until the specified frame. Implementation: `src/drop_lifetime.rs`,
`src/cash_protection.rs` and `src/pickup_probe.rs`.
