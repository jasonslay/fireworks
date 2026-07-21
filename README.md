# Fireworks

A realistic fireworks simulator written in Rust with [Bevy](https://bevyengine.org/).

## Run

```bash
cargo run --release
```

## Controls

| Input | Action |
|-------|--------|
| Left click | Launch a shell that bursts at the clicked point |
| Space | Finale salvo (8 shells at once) |
| A | Toggle automatic launching |
| F11 | Toggle borderless fullscreen |
| Esc | Quit |

The window is freely resizable; the scene scales uniformly to fit (a fixed
1280x800 virtual view, with extra sky and sides revealed on larger screens).

## What makes it look real

- **Spherical bursts** – stars are sampled on a 3D sphere and projected to the
  screen, reproducing the dense-rimmed silhouette of real shell breaks.
- **Pyrotechnic colors** – palettes based on real emitters (strontium red,
  barium green, copper blue, sodium gold, magnesium silver), with white-hot
  ignition fading through the star's color into a dim orange ember.
- **Seven shell types** – peony, chrysanthemum, willow, palm, ring, crossette
  (stars that split mid-flight), and strobe.
- **Physics** – gravity, per-star aerodynamic drag, and a slowly wandering wind.
- **HDR + bloom** – particles render at HDR intensities through a soft radial
  texture and a bloom pass, so bright stars genuinely glow.
- Detonation flash, sparky propellant tails on rising shells, burn flicker,
  twinkling background stars, and a moonlit night sky.
