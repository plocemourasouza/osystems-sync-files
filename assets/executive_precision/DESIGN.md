---
name: Executive Precision
colors:
  surface: '#101319'
  surface-dim: '#101319'
  surface-bright: '#363940'
  surface-container-lowest: '#0b0e14'
  surface-container-low: '#191c22'
  surface-container: '#1d2026'
  surface-container-high: '#272a30'
  surface-container-highest: '#32353b'
  on-surface: '#e1e2eb'
  on-surface-variant: '#c2c7d0'
  inverse-surface: '#e1e2eb'
  inverse-on-surface: '#2d3037'
  outline: '#8c919a'
  outline-variant: '#42474f'
  surface-tint: '#9ecafe'
  primary: '#9ecafe'
  on-primary: '#003257'
  primary-container: '#6794c5'
  on-primary-container: '#002b4c'
  inverse-primary: '#326190'
  secondary: '#ebbf89'
  on-secondary: '#452b02'
  secondary-container: '#5f4116'
  on-secondary-container: '#d8ae79'
  tertiary: '#85d7ae'
  on-tertiary: '#003824'
  tertiary-container: '#4fa07b'
  on-tertiary-container: '#00311f'
  error: '#ffb4ab'
  on-error: '#690005'
  error-container: '#93000a'
  on-error-container: '#ffdad6'
  primary-fixed: '#d1e4ff'
  primary-fixed-dim: '#9ecafe'
  on-primary-fixed: '#001d35'
  on-primary-fixed-variant: '#134976'
  secondary-fixed: '#ffddb6'
  secondary-fixed-dim: '#ebbf89'
  on-secondary-fixed: '#2a1800'
  on-secondary-fixed-variant: '#5f4116'
  tertiary-fixed: '#a1f4c9'
  tertiary-fixed-dim: '#85d7ae'
  on-tertiary-fixed: '#002114'
  on-tertiary-fixed-variant: '#005237'
  background: '#101319'
  on-background: '#e1e2eb'
  surface-variant: '#32353b'
typography:
  display:
    fontFamily: hankenGrotesk
    fontSize: 2rem
    fontWeight: '600'
    lineHeight: 2.5rem
    letterSpacing: -0.025em
  headline-lg:
    fontFamily: hankenGrotesk
    fontSize: 1.5rem
    fontWeight: '600'
    lineHeight: 2rem
    letterSpacing: -0.02em
  headline-md:
    fontFamily: hankenGrotesk
    fontSize: 1.25rem
    fontWeight: '500'
    lineHeight: 1.75rem
    letterSpacing: -0.015em
  headline-sm:
    fontFamily: hankenGrotesk
    fontSize: 1.125rem
    fontWeight: '500'
    lineHeight: 1.5rem
    letterSpacing: -0.01em
  title-md:
    fontFamily: hankenGrotesk
    fontSize: 0.9375rem
    fontWeight: '500'
    lineHeight: 1.375rem
    letterSpacing: -0.005em
  body-lg:
    fontFamily: hankenGrotesk
    fontSize: 0.9375rem
    fontWeight: '400'
    lineHeight: 1.5rem
    letterSpacing: 0em
  body-md:
    fontFamily: hankenGrotesk
    fontSize: 0.8125rem
    fontWeight: '400'
    lineHeight: 1.25rem
    letterSpacing: 0em
  body-sm:
    fontFamily: hankenGrotesk
    fontSize: 0.75rem
    fontWeight: '400'
    lineHeight: 1.125rem
    letterSpacing: 0.01em
  mono-data:
    fontFamily: jetbrainsMono
    fontSize: 0.8125rem
    fontWeight: '400'
    lineHeight: 1.25rem
    letterSpacing: -0.01em
  label-md:
    fontFamily: jetbrainsMono
    fontSize: 0.6875rem
    fontWeight: '500'
    lineHeight: 1rem
    letterSpacing: 0.06em
  label-sm:
    fontFamily: jetbrainsMono
    fontSize: 0.625rem
    fontWeight: '500'
    lineHeight: 0.875rem
    letterSpacing: 0.08em
rounded:
  sm: 0.125rem
  DEFAULT: 0.25rem
  md: 0.375rem
  lg: 0.5rem
  xl: 0.75rem
  full: 9999px
spacing:
  unit-2xs: 0.125rem
  unit-xs: 0.25rem
  unit-sm: 0.5rem
  unit-md: 0.75rem
  unit-lg: 1rem
  unit-xl: 1.5rem
  unit-2xl: 2rem
  unit-3xl: 3rem
  sidebar-width: 16rem
  inspector-width: 20rem
  table-row-compact: 1.75rem
  table-row-regular: 2.25rem
---

## Brand & Style
The design system targets enterprise executives, system architects, and operations leads overseeing mission-critical data sync operations. The aesthetic evokes the deliberate, hyper-focused restraint of bespoke avionics instruments and specialized financial terminal software. 

The philosophy merges extreme functional minimalism with high-craft desktop tactile precision. Rather than treating darkness as an absence of light, the interface uses measured tonal steps of deep charcoal and cold basalt slate to create visual density, stillness, and institutional trust. Visual weight is communicated strictly through micro-hierarchies, rigorous alignment, surgical typography, and whisper-soft structural separation. No decorative embellishments, no saturated neon accents, and zero high-gloss gradients are permitted; every pixel serves signal clarity, high information density, and low optical fatigue during continuous observation.

## Colors
The palette is built exclusively around matte, desaturated pigment structures suspended over obsidian and slate foundations.

- **Primary (`#4f7cac` - Muted Ice Slate):** Serves as the primary operational signal. Used sparingly for active focus indicators, critical interactive states, and selected navigation nodes. It avoids vibrant synthetic electric blues in favor of cold, mineral-derived steel.
- **Secondary (`#b08958` - Burnished Bronze):** Functions as the secondary tier for warning thresholds, remote synchronization states, and secondary telemetry metadata.
- **Tertiary (`#4e9f7a` - Muted Sage Emerald):** Applied strictly to nominal states, verified parity signals, and operational sync status. It reads as natural patinated mineral rather than high-saturation green.
- **Neutral & Surface Hierarchy:**
  - Base Obsidian Background: `#0d0f12`
  - Mid Slate Canvas: `#12151b`
  - Elevated Container / Toolbars: `#181c24`
  - Active / Hover Plate: `#1e232e`
  - Structural Ghost Borders: `#222733`
  - Subtle Hairline Divider: `#1c202a`
  - Text Primary: `#d6d9e0` (softened titanium white, avoiding harsh 100% white)
  - Text Secondary: `#8a92a3` (slate gray)
  - Text Tertiary / Disabled: `#535a6b` (recessed slate)

## Typography
Typography is split between two deliberate engines:
1. **Primary Interface Type (`Hanken Grotesk`):** Delivers clean neo-grotesque authority with calibrated geometric proportions, rendering clear hierarchies across executive titles, summaries, and dialog bodies.
2. **Telemetry & Meta Type (`JetBrains Mono`):** Dedicated to technical readouts, byte rates, checksums, timestamps, and column header micro-labels.

All uppercase labels must utilize expanded letter-spacing (`0.06em` to `0.08em`) to preserve legibility at micro-scales. Text contrast never hits absolute `#ffffff`; instead, `#d6d9e0` avoids harsh screen glare, ensuring executive comfort during prolonged analytical sessions.

## Layout & Spacing
The layout architecture is structured for an executive-grade desktop multi-pane cockpit:
- **Structural Model:** Fixed-pane shell with persistent high-density navigation, an expandable/collapsible contextual inspector, and a fluid data workspace.
- **Micro-Grid:** Built on a rigorous 4px baseline grid with an 8px macro component cadence. 
- **Density Tiers:** Density favors compact efficiency. High-data zones like telemetry logs and file synchronizers use `table-row-compact` (28px height) and 8px horizontal padding, while operational summaries expand to `table-row-regular` (36px).
- **Responsive Adaptations:**
  - **Desktop Wide (>1440px):** Three-column configuration (Navigation Pane 256px, Primary Sync Canvas Fluid, Detail Telemetry Pane 320px).
  - **Compact Desktop / Small Screen (1024px - 1439px):** Inspector defaults to an overlay drawer or tabbed sheet; table cells drop secondary hash signatures to maintain primary path clarity.

## Elevation & Depth
Depth in this system is non-skewer and non-photomorphic. It is created through **tonal stacking** and **micro-hairline borders**:

- **Stacking Tiers:**
  - **Level 0 (Canvas Base):** `#0d0f12` – The root app viewport and window gutters.
  - **Level 1 (Sub-Panels / Tables):** `#12151b` – Main operational areas.
  - **Level 2 (Cards & Inset Controls):** `#181c24` – Grouped modules, cards, and input backgrounds.
  - **Level 3 (Floating Menus / Modals):** `#1c202a` – Menus, dropdown context flyouts, and command bars.

- **Borders & Outlines:**
  Every surface boundary is framed with a 1px solid stroke in `#222733`. For sub-elements and grid row separators, a fainter `#1c202a` stroke is used. No multi-layer blurry drop shadows are permitted; popovers and modals use a crisp `0 8px 24px rgba(0, 0, 0, 0.65)` shadow strictly paired with a structural outline to maintain absolute edge definition against dark backgrounds.

## Shapes
Geometry is disciplined, architectural, and compact (`Soft` - level 1):
- Base interactive elements (buttons, inputs, status badges) use an exact corner radius of `0.25rem` (4px).
- Cards, modal containers, and floating panels use `0.375rem` (6px) or `0.5rem` (8px maximum).
- Circular treatment is reserved exclusively for unread indicator pips and pulse points (status LEDs).

This restraint avoids the consumer casualness of pill shapes and the aggressive brutality of completely unrounded 0px corners, producing a technical, tool-like precision.

## Components

### Buttons
- **Primary:** Background `#4f7cac`, text `#0d0f12` (bold), border `1px solid #6495ed33`. Hover: `#5a8dee`. Active: `#3f6a99`. Provides unmistakable confirmation without neon luminosity.
- **Secondary / Neutral:** Background `#181c24`, text `#d6d9e0`, border `1px solid #222733`. Hover: `#1e232e`, border `#32394a`.
- **Destructive / Alert:** Background `#231718`, text `#e27a7a`, border `1px solid #4a2424`.
- **Dimensions:** Compact 28px height for toolbars; 32px height for modal actions.

### Badges & Status Indicators
- Status components consist of a 6px solid dot indicator paired with an uppercase JetBrains Mono micro-label.
- **Synchronized (Nominal):** `#4e9f7a` dot, background `rgba(78, 159, 122, 0.08)`, text `#4e9f7a`.
- **In-Progress / Pending:** `#b08958` dot, background `rgba(176, 137, 88, 0.08)`, text `#b08958`.
- **Idle / Archived:** `#535a6b` dot, text `#8a92a3`.

### Input Fields & Selectors
- Height: 30px standard desktop field.
- Default: Background `#12151b`, border `1px solid #222733`, font `hankenGrotesk` 13px, text `#d6d9e0`.
- Focus: Border `1px solid #4f7cac`, subtle glow `0 0 0 1px #4f7cac33`, background `#181c24`.
- Placeholder: Text `#535a6b`.

### Lists & Data Tables
- Row separation via `1px solid #1c202a`.
- Header: JetBrains Mono 11px uppercase (`#8a92a3`), tracked out `0.06em`, background `#0f1217`.
- Hover state: Row background subtly shifts to `#161922` with a 2px vertical accent bar on the leading edge for active selection.

### Cards & Container Panels
- Surface `#12151b` wrapped in a 1px border of `#222733`.
- Internal sections are separated by structural lines rather than white space gutters to optimize information layout and visual rigor.

### Telemetry Trackers / Progress Meters
- Background groove: 4px height, color `#181c24` with `1px solid #222733`.
- Fill track: Solid `#4f7cac` or `#4e9f7a`, transitions smoothly without spring or bounce physics.