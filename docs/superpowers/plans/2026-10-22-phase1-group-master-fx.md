# Phase 1 — Group and master effects (design and plan)

> Roadmap: `docs/superpowers/plans/2026-10-22-parity-roadmap.md`. Executed inline
> (superpowers:executing-plans), test-first.

## Why
Projects store effects on groups and the master (Maschine's MASTER / GROUP chains),
but DECKARD plays only sounds' chains: group and master effects are silent, and Perform
FX (Phase 5) live on groups.

## How (decided)
- **Model:** `Project::master_slots` (the song's chain), as `Group::slots`.
- **Engine — groups:** a group whose chain has effects gets a bus. Routes stop there:
  a sound's gain is what comes before the group's input (its own fader, any bus
  sounds on the way), and it mixes into the group's bus buffer. After a span's sounds,
  each group bus runs its chain, then its own fader, onto where the group goes (the
  master's output, an external output, the cue bus; a group into another group's bus is
  not a Maschine route). Groups without effects stay folded into their sounds' gains
  as now, so projects without group effects render bit for bit as before.
- **Engine — master:** the master's level and pan are applied once per span to the
  master output already; its chain runs on that buffer just before them. (Channels
  sent straight to the master's output past the master go through its effects too: a
  DECKARD simplification.)
- **Edits:** group and master chains adopt their old state on edits (as sounds'
  chains: `FxChain::adopt`), fade units in and out, carry plug-in instances
  (`Slot::Group`, `Slot::Master` in the plug-in host's keys).
- **Rendered ahead:** sounds rendered ahead mix into the bus like the others (their
  gains are computed the same way).
- **Document:** chain edits (`add_slot`, `remove_slot`, `move_slot`, `set_slot_active`,
  parameters, plug-in chunks, presets) take a chain owner — `Chain::Sound(g, s)`,
  `Chain::Group(g)`, `Chain::Master` — instead of `(g, s)`.
- **Software:** the chain area's MASTER / GROUP / SOUND tabs show and edit that chain
  (the + menu offers effects only for groups and the master); the Mix view's strips
  show each channel's effects.
- **MK3:** PLUG-IN's MASTER / GROUP buttons show that chain; loading, removing,
  bypassing and the knobs work there.

## Steps (each test-first)
1. Model: `master_slots` read from the song (test: a project with master effects).
2. Engine routing: `first_bus` — routes stop at a group with effects (unit tests of
   gains: with and without group effects; muted group; group to Ext; cue).
3. Engine group buses: a group's internal effect (e.g. a Filter or Distortion)
   changes the group's sum and not a sound alone; bit-identical render without group
   effects (existing tests); rendered-ahead sounds through a group bus match inline.
4. Engine master chain: a master effect changes the output; level after it.
5. Engine edits: adopting an edit keeps a group effect's state (no click: continuous
   output across the edit); adding/removing a group effect fades.
6. Plug-in host keys for group/master plug-ins (VST3 on a group plays).
7. Document: `Chain` owner for chain edits; group and master slot add/remove/move/
   bypass/params, saved and reopened.
8. Software: chain area MASTER/GROUP tabs edit those chains; Mix strips list them.
9. MK3: PLUG-IN MASTER/GROUP show and edit them (lua-host test + sweep).
10. Docs (`docs/engine-design.md` section, PARITY), full check.
