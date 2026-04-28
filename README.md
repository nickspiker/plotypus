# Plotypus

**A 2D (and eventually 3D) graphing calculator self-hosted on [Spirix](https://github.com/nickspiker/spirix), with chrome lifted from [Photon](https://github.com/nickspiker/photon).**

---

## What This Is

Plotypus plots functions over the Spirix number system instead of IEEE-754. The point is that values which leave the representable range — singularities, divergent series, escapes, undefined operations — keep their structure instead of collapsing to `NaN` or `±inf`. A function that runs off to infinity in one direction still draws as that direction; a domain-coloured complex plot stays oriented past the edge of the world; a region of the plane that is unrepresentable tells you *why* via colour, not just *that* it is.

## Status

Early scaffold. Window, decoration-less chrome, compositing, and input wiring are up. The expression parser and plot pipeline are being ported from [basecalc](https://github.com/nickspiker/basecalc) onto Spirix arithmetic; until that lands there is nothing to evaluate.

## Number Formats

Plotypus accepts any Spirix numeric type — pick whatever fraction `F` and exponent `E` widths suit the curve you're after. The defaults, and the formats the rendering pipeline is tuned for, are **Circle** and **Scalar**:

- **`Scalar<F, E>`** — real numbers. Escaped values (Exploded ↑, Vanished ↓) preserve sign past the exponent range, so a function diverging to ∞ still draws with the correct orientation.
- **`Circle<F, E>`** — complex numbers with a shared exponent across real and imaginary parts. Escapes preserve angle, so domain-coloured plots remain coherent past the edge.

Both expose Spirix's full undefined taxonomy — zero, general undefined, vanished, exploded, plus typed undefined states that record *which* operation produced them — instead of folding everything into a single `NaN`. Plotypus colour-maps each of these distinctly, giving you escape-coloured, undefined-channel plots for free. Other Spirix widths work; you just lose those channels if the type can't carry the information.

## Building

`cargo build --release`

Linux and macOS only at the moment. Plotypus inherits Photon's `softbuffer` fork via `[patch.crates-io]` for Wayland dirty-region work; the macOS path still has a manual-resize TODO inherited from the same lift.

## Layout

- `src/ui/` — chrome, compositing, input, lifted from Photon
- `src/main.rs` — winit shell and event loop
- evaluator + plot pipeline land in `src/` once the basecalc port is far enough along

## License

See [LICENSE](LICENSE). Plotypus is free for personal, educational, research, and internal commercial use, with explicit royalty-free permission for hardware embodiments (FPGA, ASIC, custom silicon). Sale of Plotypus or any substantive component as the principal good requires a separate commercial license — contact below.

Plotypus depends on [Spirix](https://github.com/nickspiker/spirix), which is licensed separately under its own terms.

## Contact

Nick Spiker — fractaldecoder@proton.me
