# Visualization guide

[Back to the showcase](../README.md#visualization-gallery) · [Audio analysis](audio-analysis.md)

Eight scenes, each with curated looks that combine palette, exposure, bloom and motion.

**Automatic camera orbit:** 3D columns, Waterfall, Fountain and Fireworks offer
**Display → Camera → Helicopter orbit**. The camera makes a complete circle around the scene,
rising gently into an elevated view with a slight bank and a slowly varying distance.
The option starts off and is saved separately for each scene, including its **seconds per orbit**
setting (20–180, default 60). Flight continues through silence. Dragging, panning or zooming
pauses flight for four seconds, then it eases back into motion. Double-click resets the camera
and pauses before resuming. Turning orbit off eases out at the current heading. **Motion = 0**
freezes the flight, and hiding the window pauses it until the window reopens.

- **3D columns** - segmented LED cores in glass housings on a glossy floor. Optional **Sparks** (Display
  menu, on by default): each lit column sheds embers from the top of its strip in proportion to its
  level, and every drum hit throws a fan of sparks from the column at its place on the frequency axis (kicks
  from the bass columns, snares from the middle, hats only when accented), bigger the harder the hit; they arc and
  skitter on the floor.
- **Fountain** - sparks fly where the volume is rising. Every frequency reports how steeply its level is
  climbing right now, and a line of emitters along a glass floor throws sparks from the frequencies that
  are rising fastest: bass on the left, treble on the right. A kick is a steep rise in the lows, so it
  throws sparks on the left; a snare is a steep rise up high, so it throws them on the right; steady sound
  throws none. The steeper the rise, the more sparks and the bigger, brighter and higher they fly. Sparks
  arc, bounce on the floor and are mirrored in it. A rise is judged against the music's own hardest recent
  hits, so one setting works across recordings and volumes. Display menu: **sensitivity** sets how close
  to those a rise must get (turn it up for more sparks, down for only the hardest hits) and **density**
  scales the count.
- **Waterfall** - the spectrum history as a wireframe terrain (mixed or stereo overlay), inside glass
  walls on all four sides. The front wall carries the live spectrum line; each side wall carries the
  terrain's edge and the back wall the oldest row, so their lines follow where the waterfall meets the
  glass. It stands on a glass floor under a night sky: stars, a nebula band, a ringed planet, a small
  planet and moons, all tinted from the theme. Bass hits flare the stars and planet atmospheres and send
  a shockwave across the sky, and the floor mirrors all of it, along with the terrain and walls. A grid
  is etched into the surface at zero level, and the walls cast a little light onto the glass, with
  sheen from overhead light bars.

- **Ferrofluid Orb** - a glossy black sphere floating over a polished floor, with broad elastic lobes under
  192 fine spikes. The spikes sit on a spiral from the bottom of the sphere to the top, one per frequency of the
  spectrum, so a rising sweep climbs the orb; each is a spring that its frequency's level pulls up and that a hit
  throws out and lets settle. Eleven soft lobes, one per EQ band, bulge with their band's level (the volume stays
  constant: a louder band swells while a quieter one sinks) and wobble on onsets, and sustained bass makes the whole
  orb breathe. The surface is a black mirror: what it shows is a studio of strip lights in the theme's colours (a
  long one overhead, two tall ones at the sides, two behind it for the rim) that slowly drifts around it. The floor
  mirrors the orb and carries the ring of eleven LEDs it floats over. Each LED anchors a curved floor spike with a rounded tip,
  bass at the left and treble at the right, whose height follows its band's level with the same attack and fall
  as the 3D columns. The floor mirrors these spikes too; bass hits send a ripple out from the ring. In silence
  the floor spikes retract and the orb settles into a calm glass
  ball. Display menu: **spikes** (a smooth lobed blob to a hedgehog); **Motion** is how elastic the fluid is (spikes
  thrown out by hits, wobbling lobes, breathing, the lights' drift; 0 gives a fluid that only follows the loudness
  in still light), **Flash** is the pulse of the lights and the glow of the spike tips.

- **Aurora** - broad translucent curtains of light standing over a dark lake, under stars and behind a low
  treeline. Six vertical sheets (fewer on a narrower picture: five at 21:9, four at 16:9) stand at different
  distances; screen x is frequency on every one (bass at the left). The nearest shows the newest spectrum and
  each farther one a little older, so a hit appears near and echoes back into the distance. Each column's height
  follows its frequency's level, the mids fold the sheets in depth (a fold turned edge-on to you shows as a bright
  pleat), the sharp lower edges burn brightest, and a hit runs a small highlight up the rays where its energy
  was. The slow energy trend spreads the stack apart; in silence the curtains settle to thin, dim lines. The lake
  mirrors the sky and ripples with the bass. The curtains are drawn at half resolution (about 0.4 ms at 5120x1440)
  and the stars, treeline and lake at full resolution. Display menu: **curtains** (the most standing in the sky,
  2-6); **Motion** is how much the curtains fold and drift and how the lake stirs (0 holds the folds still; the
  columns still follow the music), **Flash** is the highlights that run up the rays.

- **Fireworks** traces the spectrum with 24 comet trails above the glass. Bass is on the left and
  treble on the right. A held note keeps its comet aloft, and a sharp attack makes it jump and brighten.
  Detected drums burst immediately above their frequency. Kicks open into wide **willows**, snares into
  round **peonies**, and hats into brief **crackles**. A strong note attack can open a smaller shell
  when the drum detector hears no drum. Harder hits burst higher and throw more stars. In a fast passage,
  each shell sheds fewer stars so the next beat stays visible. The trails settle and the sky clears in silence.
  **Sensitivity** adjusts which drums and note attacks trigger shells. **Density** sets the number of
  stars, **Motion** sets wind drift and camera movement, and **Flash** controls the central flash and floor light.

- **Lightning Storm** - each hit strikes the glass floor as a bolt of branching light, where it landed on the line
  (kicks on the left, snares in the middle, hats on the right; the **sensitivity** setting controls how confident the drum detector must be).
  Every bolt is built on the GPU from its own seed by midpoint displacement and drawn as thin glowing lines that are
  anti-aliased analytically: the first stroke runs down the channel in about 20 ms, one to three return strokes
  flicker (a harder hit flickers more), and a dim channel stays behind for a third of a second. Where the hit landed
  sets the character: lows strike thick with few branches, from the clouds above the frame, and send a **thunder ring**
  out across the floor; highs are thin, many-branched and short. The strike throws a pool of light on the glass and a
  splash of sparks, and every bolt is mirrored in the floor. Between strikes faint discharges flicker on the spectrum
  line and grow with the level. No part of the line flashes more than three times a second: a strike that finds three
  full ones already within a fifth of the line in the last second is drawn as a dim ghost, with no flash on the floor.
  Silence strikes nothing and what is up fades out. Display menu: **sensitivity** and **density** (density scales
  the branches and sparks); **Motion** is how far the camera wanders, **Flash** is the strokes' brightness and the
  light and thunder ring on the floor (0 leaves only a dim channel).

- **Ripple Pond** - a glossy dark pool that keeps a record of the last few seconds of hits. A drum hit (the same detector as
  the Fireworks and the storm) breaks the water at once where it landed on the line, bass at the left, a broad deep
  crater for the lows and a small quick one for the highs, and throws a few droplets in arcs; each droplet that comes
  back down drops a ring where it lands (a harder hit throws more of them). The surface is a damped wave equation on a
  grid of cells 1/20 of a unit wide, stepped at 240 Hz by a fragment pass per step, with a sponge around its edge so
  rings leave instead of coming back, so rings cross and interfere and the pool is a living picture of what was
  just played. Splashes are broader and deeper, and the pond accepts softer drum hits for more frequent rings.
  The water is a mirror: it shows a faint haze along the horizon and eleven light poles beyond the far shore,
  one per EQ band (bass at the left, highs at the right). Each lights from the bottom up to its band’s level,
  using the same rise and fall as the 3D columns, and flares on onsets, so every passing
  ring bends their reflections and sweeps highlights over the pool; the droplets are mirrored too, bent by the ripples
  under them. The slope the water is shaded from is mipmapped and read anisotropically, so a thin ring far away is
  averaged over the pixel's footprint instead of aliasing. In silence the rings damp out quickly and leave calm glass with slowly drifting reflections. Display
  menu: **sensitivity** and **density** (density scales the droplets), **ripple life** (how long a ring lives: short
  is a quick clean pond, long a surface crowded with hits); **Motion** is how far the towers drift and the camera
  wanders (0 holds the reflections still; the water still follows the music), **Flash** is the towers' flares and the
  droplets' glow.

