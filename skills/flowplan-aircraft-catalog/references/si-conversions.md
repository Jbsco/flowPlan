# POH → SI Conversions

Every number in a `data/aircraft/<name>.ron` that's SI was converted from a POH figure that was imperial. Doing the arithmetic on authoring (once, by a human or agent who checked sources) is the discipline; doing it at runtime is a footgun waiting for a rounding bug to invalidate a scenario.

## Exact conversion factors

| From | To | Factor | Notes |
|------|-----|--------|-------|
| mph | m/s | × 0.44704 | Common for US POHs |
| kt | m/s | × 0.51444 | Common for cruise and Vne |
| ft/min | m/s | × 0.00508 | ROC figures in US POHs |
| ft | m | × 0.3048 | Altitudes, ground roll |
| nm | km | × 1.852 | Range |
| sm | km | × 1.609 | Range (statute miles, less common) |
| lb | kg | × 0.45359 | Round to 4 sig figs for mass |
| sq ft | m² | × 0.09290 | Wing area |
| US gal | L | × 3.7854 | Fuel — keep as gal in spec for recognizability |
| hp | kW | × 0.74570 | Power — keep as hp in spec for recognizability |
| in | m | × 0.0254 | Prop diameter, misc |
| in Hg | Pa | × 3386.39 | Manifold pressure (future) |

Where the table says "keep as X in spec": the sim converts internally when needed. `power_hp` stays as `450.0` in RON; the flight model multiplies by `0.7457` to get kW.

## Rounding and significant figures

Keep 4 sig figs on conversions. More than that is false precision (the POH itself is rarely better than 3 sig figs). Less than that rounds out useful resolution on stall speeds.

Examples:
- 52 mph × 0.44704 = 23.2461 → **23.25 m/s**
- 47 mph × 0.44704 = 21.0109 → **21.01 m/s**
- 5,100 lb × 0.45359 = 2313.31 → **2313.3 kg**
- 250 sq ft × 0.0929 = 23.225 → **23.23 m²**

## Worked example: the Beaver

From the POH:

```
MTOW (wheels):  5,100 lb
MTOW (floats):  5,370 lb
Empty:          2,900 lb
Wing area:      250 sq ft
Aspect ratio:   9.2
Engine:         R-985, 450 hp
Cruise fuel:    27 gph
Fuel capacity:  95 gal
Vne:            160 mph
Cruise:         125 kt
Vs (clean):     52 mph
Vso:            47 mph
Vx:             65 mph
Vy:             85 mph
```

Becomes:

```ron
// 5,100 lb wheels MTOW -> 2313.3 kg
max_mass_wheels_kg: 2313.3,
// 5,370 lb floats MTOW -> 2435.8 kg
max_mass_floats_kg: 2435.8,
// 2,900 lb empty -> 1315.5 kg
empty_mass_kg: 1315.5,
// 250 sq ft -> 23.23 m^2
wing_area_m2: 23.23,
aspect_ratio: 9.2,
// hp and gal retained for operator recognizability
engine: (name: "Pratt & Whitney R-985 Wasp Junior", power_hp: 450.0, cruise_fuel_gph: 27.0),
fuel_capacity_gal: 95.0,
// Vne   160 mph -> 71.53 m/s
// cruise 125 kt -> 64.31 m/s
// Vs     52 mph -> 23.25 m/s
// Vso    47 mph -> 21.01 m/s
// Vx     65 mph -> 29.06 m/s
// Vy     85 mph -> 37.99 m/s
speeds: (
    vne_mps: 71.53,
    cruise_mps: 64.31,
    vs_clean_mps: 23.25,
    vso_mps: 21.01,
    vx_mps: 29.06,
    vy_mps: 37.99,
),
```

Note the comment discipline: every SI figure has a comment stating the original POH figure. This makes review possible.

## Unit traps

- **mph vs kt.** Beaver's Vne is 160 mph; its cruise is 125 kt. Mixing units from the same POH is a real source of error. *Always* verify the unit per-field.
- **MTOW wheels vs floats.** Floats are heavier (extra structure + float mass). Some POHs only publish one; if only one exists, note which one and leave the other field copied with a comment.
- **Usable vs total fuel.** Beaver has 95 usable of ~100 total. Always use *usable* — total-capacity fuel is misleading because the unusable is reserve.
- **Rated vs takeoff power.** The R-985 is 450 hp at 2,300 RPM (takeoff). Cruise power at 65% is ~293 hp. The spec stores rated (takeoff); the engine model's power-curve function derates internally.
- **Density altitude figures.** POHs often publish Vs/Vso as "IAS" (indicated). The sim works in TAS (true airspeed). At SL they're equal; at altitude TAS > IAS. Scenarios that stall-test at altitude need to account for this.

## When a POH figure is missing

Some aircraft don't have modern POHs (out-of-production, golden-age). In order of fallback:

1. **TCDS (Type Certificate Data Sheet).** FAA / Transport Canada; gives limits but not all performance.
2. **Manufacturer technical bulletins.** Viking (Beaver/Otter/Twin Otter), Daher (Kodiak), Aviat, CubCrafters, Maule are good.
3. **Operator-community knowledge.** SuperCub.org, Backcountry Pilot, operator retrospectives.
4. **Reconstruction from related variants.** A DHC-2T is a PT6-converted Beaver; much carries over except power and burn.

Tag `(verify)` on anything fallback-sourced; keep a wider tolerance on any scenario depending on it.
