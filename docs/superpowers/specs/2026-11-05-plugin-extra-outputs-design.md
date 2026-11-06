# A plug-in's extra outputs in the mixer

Proposed as a replacement for PR #23 ("Let a plug-in on the master feed a second pair
of outputs").

## Why

Some plug-ins have more than one stereo output: a main pair plus extra pairs (Kontakt's
multi-outs, Portal, Element, drum samplers that put each drum on its own pair). PR #23
lets a plug-in on the master send its first extra pair to the output after the master's.
That works, but:

- **Nobody would know it's there.** Nothing in the app shows the second pair exists, where
  it goes, or that anything happened.
- **One fader runs both pairs.** The second pair takes the master's level and pan, so it
  can't be balanced on its own.
- **It's narrow.** Only a plug-in on the master, only its first extra pair, and only to
  the next output. That's four outputs at most.

The goal is a feature people can find just by using a plug-in that has extra outputs.

## What it does

### Extra outputs appear when sound comes out of them

- Deckard reads how many output pairs each plug-in has, VST3 and Audio Unit alike, on any
  chain: a sound's, a group's or the master's.
- An extra pair does **not** get a fader just because the plug-in has it. A plug-in like
  Kontakt reports 16 pairs, and 15 empty faders would be clutter. The fader appears **the
  first time sound comes out of that pair**.
- Once it has appeared, it **stays**, and it is saved with the project, so faders don't
  come and go while you play. When the plug-in is removed or replaced, its extra faders go
  with it.
- **Every** extra pair the plug-in has can appear this way, not just the first.

### Each extra output is its own mixer strip

In the mixer, each extra output gets a strip **in the groups mixer, just left of the
Master**, whoever owns the plug-in: a sound, a group or the master. They never show in a
group's sounds mixer. (The user moved them here on 2026-11-06; the first design put each
strip next to its owner.) They are ordered by owner in mixer order: groups A, B, …
(a group's own plug-ins, then its sounds' in sound order), then the master's. For Portal on
Group A with two extra pairs and Kontakt on sound A3 with one:

```text
| Group A | Group B | ... | A·Portal | A·Portal | A3·Kontakt | Master |
|         |         |     | Out 2 ▾  | Out 3 ▾  | Out 2 ▾    |        |
```

- Because a strip no longer sits by its owner, its top line names the owner too: the
  owner's ID (group letter, "A3" for a sound, "M" for the master) and the plug-in's name,
  e.g. "A3·Kontakt". Hovering shows the owner's full name.

- These strips are called **Outs**, never "Aux": in Deckard, Aux already means the send
  effects.
- The strip is labelled with the plug-in's name and the output's number, "Out {bus+1}"
  (the main pair is Out 1). When the plug-in gives the bus a real name, such as "Kick",
  that name is shown as well. Generic names such as "Aux 1", "Out 3", "Output 3/4" or
  "St. 3/4" are hidden, since the number already says the same thing.
- It looks a little different (tinted), so it reads as a plug-in's output and not as
  another group.
- It has its own **level, pan, mute, solo and meter**, independent of the channel that has
  the plug-in. In particular, the master's fader no longer moves it, unlike #23.

### Stereo, mono or split

Every strip starts as **stereo**. A menu on the strip changes it (the user decided this on
2026-11-06):

- **Stereo:** as described above.
- **Mono:** left and right are summed into one mono signal, (L+R)/2, played equally on
  both sides. A mono strip has **no pan** (the user decided this on 2026-11-06); its pan
  control is hidden and any saved pan is ignored.
- **Split:** the strip becomes two mono strips, **L** and **R**, labelled
  "Portal · Out 3 L" and "Portal · Out 3 R". Each half is mono (its one side played
  equally on both sides, with no pan, like a Mono strip) and has its own level, mute, solo,
  meter and output, so a drum plug-in with the kick on the left and the snare on the right
  of one pair can be mixed as two channels. Both halves start with the split strip's
  settings. On a split half, the menu offers **Join**, which turns the pair back into one
  stereo strip with the L half's settings.

Changing the mode is one undoable edit, and it is saved.

### Where an extra output's sound goes

- **Default:** the next hardware output that nothing else is using. With the master on
  Out 1 and one extra output already on Out 2, the next one takes Out 3.
- **The strip's output picker** changes it, offering the same places a channel's Output can
  go today (`AudioDest`): a hardware output, the master, the channel above (its parent), a
  group, or a sound. That last one includes a sound that takes audio input, so the extra
  output can be processed again inside Deckard.
- **No free hardware output** (for example a two-output interface): it goes to the channel
  above (its parent) so it's still heard. The picker shows that's where it went.

### Exports

When the song is exported, each extra output that goes to its own hardware output is
**saved as a separate stem**, named after its strip (for example "Group A - Portal Out 2").
A mono or split-half strip exports as a mono signal on both channels of its stem.
The master file stays what comes out of the master. An extra output routed back into
Deckard (to the master, a group or a sound) is already part of whatever it feeds, so it
gets no stem of its own. (The user decided this on 2026-11-05.)

### What happens to the main pair

The plug-in's main pair carries on through the rest of its chain exactly as it does now.
Plug-ins with one pair behave exactly as they do now, and a project with no extra outputs
in use saves exactly as before.

## Non-goals

- **Extra inputs** (side-chain buses and multi-input plug-ins) aren't part of this.
  Side-chain already has its own feature.
- **Mono or surround extra buses** aren't needed beyond what's simplest to support. The
  target is stereo pairs. A mono bus may be shown as a stereo strip with the same signal
  on both sides, or left out (builder's call).
- **Effects on the extra strips.** They are level, pan and routing only. To process one,
  route it to a sound or group that has effects.
- **Maschine compatibility** for these strips: Maschine has nothing to save them in, so
  they go in the DECKARD block (`docs/deckard-block.md`). Maschine opens the project and
  ignores them, as it already does for MPE and Pad Touch.

## Anti-goals (what would make this a failure even if it works)

- A strip appears with nothing to tell the user where it came from.
- Opening a Kontakt multi-out patch fills the mixer with silent strips.
- The audio thread allocates, locks or does per-block work that grows with the number of
  pairs a plug-in *could* use, rather than the ones in use. Detecting sound runs on the
  audio thread, so it must be cheap and must never allocate.
- Projects that don't use extra outputs sound or save any differently.

## Decisions taken

These were decided by the user (Josh) in the 2026-11-05 brainstorm:

- Detect extra outputs automatically, with no setup and no hidden behavior.
- Show a strip only once sound comes out of that output.
- Put the strip next to the channel that has the plug-in. (Changed 2026-11-06: all strips
  sit in the groups mixer, left of the Master; none in a group's sounds mixer.)
- Give it its own fader, independent of the master's.
- By default route it to the next free hardware output, changeable in the mixer, including
  back into Deckard's own channels.
- On export, an extra output on its own hardware output becomes a separate stem.
- (2026-11-06) A strip can be stereo, mono, or split into two mono strips (L and R).
- (2026-11-06) Don't call them "Aux"; that name already belongs to the send effects.

These are Claude's calls, which the user hasn't confirmed separately:

- It works on any chain (sound, group or master), not only the master.
- A strip stays once it has appeared, and is saved.
- With no free hardware output it falls back to the parent channel.
- The strips are saved in the DECKARD block.
- **Detection cost:** every output bus a plug-in has is switched on (many VST3s leave aux
  buses off by default), and each extra bus gets a cheap peak check every block **until
  its strip has appeared**. After that, the strip's meter does the job. This is the
  accepted cost. The anti-goal means no allocating or locking, and no ongoing work on
  buses nobody uses beyond that check. The plug-in host's stereo-only
  `process_input` grows a way to read the extra buses.
- **Default output is fixed when the strip appears, and saved** as an ordinary output
  choice. Strips never move on their own. If a saved hardware output doesn't exist on the
  interface in use (a project reopened on a smaller interface), it plays through the parent
  channel, and the picker says so, the same fallback as when there's no free output. Two
  strips appearing in the same block take free outputs in mixer order.
- **Solo:** an extra strip solos like any channel. Soloing it silences everything else,
  including its owner's main pair, while the plug-in keeps running. Soloing something
  else silences it, unless that something is its owner, because soloing a channel keeps
  everything that comes from it.
- **A sound's plug-in:** its extra strips' parent is that sound's group (for the fallback
  route). They show in the groups mixer like every other strip.
- **PR #23** isn't merged, so no project depends on it. Nothing needs migrating. This
  design replaces it.

## Left to the builder

- **What counts as "sound came out":** a threshold such as a peak above −90 dBFS over a
  block, and how the audio thread tells the UI (for example a per-bus flag the UI reads,
  as the meters do).
- How strips are stored: keyed by the plug-in's slot plus its bus index, and how that key
  survives moving the plug-in to another slot.
- The order and position of the strips on the MK3's mixer screens, and whether the
  hardware shows them at all in the first version. The software mixer comes first.
- What happens when a strip routes to a channel that feeds back into itself. Prevent it or
  refuse it the same way existing routing handles loops.
- Whether to reuse #23's test plug-in with two output buses (`crates/plugins/src/native.rs`)
  and its engine tests. They cover much of the routing.

## Testing

- **Engine tests** with a native test plug-in that has several output buses. Cover: a
  silent extra bus makes no strip; sound on it makes one; level, pan, mute and solo
  work independently of the owning channel; the default output picks the next free
  output; the parent fallback when none is free; routing into a group and into a sound;
  removing the plug-in removes its strips; and a plug-in with one pair is unchanged.
- **Export tests:** an extra output on a hardware output becomes its own stem, and one
  routed into a group doesn't.
- **Save tests:** strips survive save and reopen, a project without them is byte-for-byte
  unchanged, and a Maschine-saved project still opens.
- **By hand:** Kontakt with a multi-out instrument and Portal (Audio Unit) on macOS, and a
  multi-out VST3 on Windows. Check that the strips appear only for the pairs actually
  playing.
