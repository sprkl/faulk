---
name: Faulk
description: Open DAW for humans and agents. A Swiss instrument dressed in grainy sound matter.
colors:
  ultraviolet: "#6633f4"
  ultraviolet-deep: "#4c1ebc"
  ultraviolet-on-dark: "#9c92ff"
  ultraviolet-tint: "#ede9ff"
  ultraviolet-tint-dark: "#2a2150"
  paper: "#fdfcfc"
  paper-surface: "#f6f4f2"
  paper-surface-2: "#ece9e6"
  paper-line: "#dad6d2"
  ink: "#131211"
  ink-surface: "#1a1917"
  ink-surface-2: "#22201e"
  ink-line: "#34312e"
  ink-text: "#ece9e6"
  muted-on-paper: "#6e6964"
  muted-on-ink: "#8f8a85"
  soft-on-paper: "#4f4b47"
  play-green: "#3fc168"
  rec-red: "#ee3533"
  solo-yellow: "#efcc36"
  mute-blue: "#4baeed"
  track-01: "#f08098"
  track-02: "#f18a76"
  track-03: "#ea9356"
  track-04: "#d99e3f"
  track-05: "#c1ab3c"
  track-06: "#a0b751"
  track-07: "#79bf72"
  track-08: "#4ac495"
  track-09: "#00c5b7"
  track-10: "#00c1d5"
  track-11: "#3bb9ed"
  track-12: "#6bafff"
  track-13: "#92a4fc"
  track-14: "#b299f3"
  track-15: "#cd90e0"
  track-16: "#e189c5"
typography:
  display:
    fontFamily: "Archivo Variable, Archivo, Arial Black, sans-serif"
    fontSize: "clamp(36px, 7.6vw, 124px)"
    fontWeight: 900
    lineHeight: 0.86
    letterSpacing: "-0.035em"
    fontVariation: "'wdth' 125"
  headline:
    fontFamily: "Archivo Variable, Archivo, Arial Black, sans-serif"
    fontSize: "clamp(28px, 4vw, 56px)"
    fontWeight: 900
    lineHeight: 0.9
    letterSpacing: "-0.03em"
    fontVariation: "'wdth' 125"
  title:
    fontFamily: "Geist Variable, Geist, Helvetica Neue, Arial, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.3
  body:
    fontFamily: "Geist Variable, Geist, Helvetica Neue, Arial, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.4
  body-site:
    fontFamily: "Geist Variable, Geist, Helvetica Neue, Arial, sans-serif"
    fontSize: "clamp(19px, 1.8vw, 24px)"
    fontWeight: 300
    lineHeight: 1.35
  label:
    fontFamily: "Geist Variable, Geist, Helvetica Neue, Arial, sans-serif"
    fontSize: "11px"
    fontWeight: 600
    lineHeight: 1.4
    letterSpacing: "0.06em"
  value:
    fontFamily: "Geist Mono Variable, Geist Mono, ui-monospace, monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.4
    fontFeature: "'tnum' 1"
rounded:
  work: "2px"
  panel: "8px"
  card: "20px"
  pill: "999px"
spacing:
  "1": "4px"
  "2": "8px"
  "3": "12px"
  "4": "16px"
  "5": "20px"
  "6": "24px"
  "8": "32px"
  "12": "48px"
  row: "28px"
  row-compact: "22px"
  lane: "44px"
  lane-compact: "28px"
  control: "32px"
  control-compact: "26px"
components:
  button-primary:
    backgroundColor: "{colors.ultraviolet}"
    textColor: "{colors.paper}"
    typography: "{typography.title}"
    rounded: "{rounded.pill}"
    padding: "0 16px"
    height: "{spacing.control}"
  button-primary-hover:
    backgroundColor: "{colors.ultraviolet-deep}"
  button-bordered:
    backgroundColor: "transparent"
    textColor: "{colors.ink-text}"
    rounded: "{rounded.pill}"
    padding: "0 14px"
    height: "{spacing.control}"
  button-icon:
    backgroundColor: "{colors.ink-surface-2}"
    textColor: "{colors.ink-text}"
    rounded: "{rounded.panel}"
    size: "{spacing.control}"
  segmented-tab-active:
    backgroundColor: "{colors.paper-surface-2}"
    textColor: "{colors.ink}"
    rounded: "{rounded.pill}"
    padding: "4px 10px"
  input-field:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.ink-text}"
    typography: "{typography.value}"
    rounded: "{rounded.panel}"
    height: "{spacing.control}"
  engine-pill-boost:
    backgroundColor: "{colors.ultraviolet}"
    textColor: "{colors.paper}"
    typography: "{typography.value}"
    rounded: "{rounded.pill}"
    padding: "4px 12px"
  engine-pill-web:
    backgroundColor: "{colors.ink-surface-2}"
    textColor: "{colors.ink-text}"
    typography: "{typography.value}"
    rounded: "{rounded.pill}"
    padding: "4px 12px"
  track-header:
    backgroundColor: "{colors.ink-surface}"
    textColor: "{colors.ink-text}"
    typography: "{typography.body}"
    rounded: "{rounded.work}"
    height: "{spacing.lane}"
  clip:
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.work}"
---

# Design System: Faulk

## Overview

**Creative North Star: "The Swiss Instrument"**

Faulk looks like a precision instrument: typographic, frank, every control where the hand expects it, labelled in plain words, with a single ultraviolet signal for what matters now. The instrument is dressed in sound matter: soft, grainy gradients that belong to sound only (a module's sound object, the landing's stage, empty states), never to chrome. The founder fixed this direction in the DS-0.1 canvas on 2026-10-13 and chose this North Star on 2026-11-01.

The system has one token set and two surfaces. The **site** (light, paper) speaks loudly: giant uppercase Archivo, airy spacing, the grain in hero. The **app** (dark, ink) disappears behind the music during sessions of several hours on large monitors: small, dense, very legible type, flat surfaces, quasi-square work areas, the sound itself (waveforms, meters, clips, playhead) as the visual hero. The app is built for experienced producers on large screens; comfort density is the default, compact is a setting.

Confirmed rejections: uppercase everywhere in the app (illegible in dense UI), decorative 3D volumes unrelated to sound, decorative motion during playback.

**Key Characteristics:**
- One accent, ultraviolet, used franchly on the site and as a rare signal in the app (selection, focus, main action, Boost).
- Warm neutrals from paper (#fdfcfc) to ink (#131211); no cold greys.
- Three families: Archivo (display, wide and black), Geist (interface), Geist Mono with tabular figures (every value: BPM, dB, Hz, positions).
- Every state has a glyph or a letter, never colour alone.
- Grain is the signature of sound, not a decoration.

## Colors

A warm, near-monochrome ink-and-paper ramp with one ultraviolet voice and a four-colour engine-state code; tracks get their own 16-hue palette.

### Primary
- **Ultraviolet** (#6633f4, oklch 0.52 0.26 285): the only accent. Flat fills on the site (sections, primary buttons); in the app only for selection, focus, the main action and the Boost state. White on it reads at 6.3:1 (DS-0.1 figure).
- **Deep Ultraviolet** (#4c1ebc): hover and pressed state of ultraviolet fills.
- **Ultraviolet on Ink** (#9c92ff): accent text, focus rings and hairlines on the dark app (7.1:1 on ink, DS-0.1 figure).
- **Ultraviolet Tint** (#ede9ff light, #2a2150 dark): quiet selection backgrounds.

### Neutral
- **Paper** (#fdfcfc), **Paper Surface** (#f6f4f2), **Paper Surface 2** (#ece9e6), **Paper Line** (#dad6d2): the light theme (site, light app option).
- **Ink** (#131211), **Ink Surface** (#1a1917), **Ink Surface 2** (#22201e), **Ink Line** (#34312e): the dark app; surfaces step up by tone, not by shadow.
- **Ink Text** (#ece9e6) on dark, **Ink** on light; **Muted on Paper** (#6e6964), **Muted on Ink** (#8f8a85) for secondary text, **Soft on Paper** (#4f4b47) for long site copy.

### Engine states
- **Play Green** (#3fc168, triangle glyph), **Rec Red** (#ee3533, disc glyph, pulsing), **Solo Yellow** (#efcc36, letter S), **Mute Blue** (#4baeed, letter M, track dimmed to 40 %). Saturation or clip: rec red with hatching and the word CLIP, latched until clicked. Boost active: ultraviolet with a lightning glyph and the latency.

### Tracks
- **Sixteen track hues** (track-01 #f08098 to track-16 #e189c5): oklch(0.74 0.13 h) with h every 22.5°, all equally bright, ink text on each at 7:1 or more. They identify tracks, never states.

### Named Rules
**The One Voice Rule.** Ultraviolet is the only accent. In the app it marks selection, focus, the main action and Boost, nothing else; if two ultraviolet things compete on one screen, one of them is wrong.

**The Glyph Rule.** No state is carried by colour alone: every engine state has its glyph or letter, the clip indicator adds hatching and a word. Red is reserved for recording and saturation, green for playback, which is why the accent is violet.

**The Track Is Not A State Rule.** Track hues name tracks; they never signal mute, solo, selection or error.

## Typography

**Display Font:** Archivo Variable (width 125, weight 900), with Archivo and Arial Black fallbacks
**Body Font:** Geist Variable, with Helvetica Neue and Arial fallbacks
**Label/Mono Font:** Geist Mono Variable with tabular figures

**Character:** A wide, black grotesque that shouts on the site against a neutral, very legible working sans; the mono carries every number so columns of values never jitter.

### Hierarchy
- **Display** (900, clamp(36px, 7.6vw, 124px), 0.86): site hero only, uppercase, the product's statement.
- **Headline** (900, up to 56px, 0.9): site section titles, uppercase.
- **Title** (600, 15px, 1.3): app panel and dialog titles, sentence case.
- **Body** (400, 13px, 1.4): all app text. Site body copy is 300 at 19 to 24px, 1.35, max about 620px wide.
- **Label** (600, 11px, 0.06em, uppercase, muted): short app labels only (section names, column heads, parameter names).
- **Value** (mono 400, 12px, tabular figures): every number and unit in the app: position, tempo, meter, dB, Hz, ms, percentages.

### Named Rules
**The Typography As Architecture Rule.** On the site, giant uppercase titles structure the page; in the app, type becomes discreet and functional, and uppercase is reserved for labels of one to three words.

**The Mono For Numbers Rule.** Any value that changes or lines up with others is in Geist Mono with tabular figures.

## Layout

The app is a fixed desktop workspace for large monitors (1440 px wide and up; a laptop is the floor; no tablet or phone layout). A top bar carries project, views, transport and the permanent engine indicator; a track column sits on the left; the main area holds the current view; a resizable bottom panel holds devices, piano roll and history. Spacing follows a 4px base (4, 8, 12, 16, 20, 24, 32, 48). Rows are 28px and controls 32px in the default comfort density, 22px and 26px in compact density (`data-density="compact"`), the setting for very large projects. A track's row in the arrangement (a lane) is its own token, `lane`: 44px in comfort, 28px in compact (`--fk-lane`), so that 16 tracks fit on a 1080p screen with the bottom panel at about 220px; each track's row can be made higher or lower by its bottom edge (28 to 176px), and the control line inside it stays at the `row` height. The master's row, which holds no clips, stays at the `row` height (28px).

The site uses a 12-column grid, generous vertical rhythm and full-bleed sections; it adapts down to phone width.

The DS-0.1 reference screen for the app is a unified view: the launchable clip slots of each track sit beside its timeline on the same row, with the device chain below and a browser of sounds and modules on the left. Today's app still splits arrangement, grid and mixer into separate views; closing that gap is a product decision tracked outside this file.

**The Room To Breathe Rule.** Professionals have large screens: show the panels they need side by side instead of hiding them behind tabs, and spend extra width on more timeline, not bigger chrome.

## Elevation & Depth

Flat by default. Depth comes from tonal steps (ink, ink surface, ink surface 2) and 1px lines, not shadows. The only shadow is the float shadow for menus, popovers and dialogs above the workspace (`0 12px 32px rgb(19 18 17 / 0.18)`).

**The Flat Workspace Rule.** Work areas (timeline, piano roll, mixer) never cast or receive shadows; only things that float above them do.

## Shapes

Four radii, each tied to a role: quasi-square 2px for work areas (clips, lanes, notes, track rows), 8px for panels, fields and icon buttons, 20px for cards (site cards, module cards), pills for actions, segmented tabs and status pills. Knobs and meters are round and vertical; the logo monogram is two flat cut shapes.

**The Square Work Rule.** Anything the user edits on a timeline or a grid is near-square (2px); roundness signals something you press, not something you edit.

## Components

Components describe the DS-0.1 target. Where today's app differs, the entry says so with **Not shipped yet** or **Today**; those gaps are design work to do, not a licence to drop the target.

### Buttons
- **Shape:** pill (999px), 32px high (26px compact).
- **Primary:** ultraviolet fill, paper text, for the one main action of a surface; hover and pressed in deep ultraviolet.
- **Bordered:** transparent with a 1px line, for secondary actions.
- **Icon buttons:** 32px square on ink surface 2 with 8px radius; toggles (loop, metronome, follow, piano-roll and typing toggles) show their on state with a neutral fill and a dot or glyph, never ultraviolet (One Voice Rule, founder 2026-11-01); Play shows its running state in play green with the triangle, Stop stays neutral.
- **Focus:** a 2px ultraviolet-on-ink outline with 2px offset, on every interactive element.
- **Disabled:** muted text on a flat surface, no fill.

### Segmented tabs
- Pills inside a bordered pill track; the active segment is a light pill with ink text (views: Arrangement, Grid, Mixer, Instrument; panel tabs).

### Inputs / Fields
- **Style:** ink fill, 1px line, 8px radius, mono values; numeric fields are draggable (drag up and down, Shift for fine).
- **Focus:** ultraviolet-on-ink border.

### Engine indicator (signature)
- A permanent pill in the top bar. Web: ink surface pill, dot, "WEB", estimated latency and CPU in mono. Boost: ultraviolet pill with a lightning glyph, "BOOST", measured latency, driver and CPU. Helper lost: amber outline with "Helper disconnected, back to Web".
- **Today:** the app shows the mode and the latency ("Web ~42 ms (estimated)") with the voice count, and the CPU load ("CPU 12 %", with the peak of the last 3 s; #594); the driver and the amber helper-lost state are not shipped yet.
- **Protection mark (#667):** a small red dot in the pill once the output protection has silenced a non-finite signal from the engine that plays; it stays while that engine plays (its count is cumulative), the tooltip and the accessible name say it in words, and the engine dialog's "Protection" row gives the duration. Apart from the yellow dot of a Boost waiting for an output.

### Track header and clips
- **Track header:** one row per track in the arrangement, at the `lane` height: the track colour as a 3px bar (also the grip that reorders the track), the name in normal case with the type and instrument under it (in the accessible name only below 40px), M, S and In toggles (letters, so state is not colour alone; In is live input, neutral inverted fill when on), and a level meter. A track silenced by another track's solo shows a dashed M (implied mute). No fader or pan in the row: the volume shows on hover and on focus, and is typed on the selected row (a digit starts the entry, Enter or F2 open it). The row's bottom edge sets its height.
- **Inspector:** the left column shows the selected track only: colour, name, type, actions (rename, move, duplicate, delete), volume, pan and a summary of its devices; it follows the selection. Fader and pan also live in the mixer.
- **Today:** the R toggle is labelled In (live input) until recording ships; the per-track height is kept for the session, not saved with the project.
- **Clips:** 2px radius, filled with the track hue at full brightness, label in ink, content (notes or waveform) drawn in darker ink inside; the selected clip gets a light outline; a recording clip is rec red with its pulse.

### Audio controls
- **Knob:** round, neutral arc (ink text) for the value, parameter label above or below in label style, value in mono; drag vertically, double-click to reset. An automated parameter shows a curve glyph and a dotted frame, never ultraviolet.
- **Fader:** vertical track with a flat cap and a neutral fill, value in mono under it; automated faders take the same curve glyph.
- **Meter:** flat green segments up to -12 dB, then wider segments so the zone reads by shape; peak hold 1.5 s as a white line; clip is a hatched top segment with the word CLIP, latched until clicked or reset from the keyboard. One meter component everywhere: each lane, each mixer strip, and the master in the top bar of every view. The Compressor card shows its gain reduction.

### Module card (signature)
- **Not shipped yet** (no module library or Atelier). 20px-radius card with the module's grainy sound object, its English name, its origin (built in, made in the Atelier, third-party plugin), and an on or bypass state in mono.

## Do's and Don'ts

### Do:
- **Do** keep ultraviolet (#6633f4) for selection, focus, the main action and Boost in the app.
- **Do** give every state a glyph or a letter in addition to its colour, and write CLIP on saturation.
- **Do** set every number in Geist Mono with tabular figures (12px in the app).
- **Do** use the 16 DS-0.1 track hues (the code still ships 8 different, older colours in `web/src/lib/project/tracks.ts`).
- **Do** design for large monitors: comfort density by default (28px rows, 44px lanes), panels side by side, extra width to the timeline.
- **Do** keep grain for sound: sound objects of modules, the site's stage, empty states.
- **Do** keep chrome still during playback; motion in the app is functional feedback under 100 ms (80ms fast, 160ms panels).

### Don't:
- **Don't** set app text in uppercase beyond one-to-three-word labels.
- **Don't** use decorative 3D volumes or grain on chrome, buttons or panels.
- **Don't** animate anything decorative in the app while the transport runs.
- **Don't** use track hues to mean a state, or red and green for anything but recording, saturation and playback.
- **Don't** add shadows to work areas; only floating layers get the float shadow.
- **Don't** name or imitate another DAW in UI copy or design notes.
