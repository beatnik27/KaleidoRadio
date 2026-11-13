# Local glutin patch

`glutin/` contains glutin 0.32.3 from the Cargo registry, including its license.
`Cargo.toml` pins it with `[patch.crates-io]` so builds consistently include this fix.

The only behavior change is in `src/api/cgl/context.rs::is_view_current`: a macOS
OpenGL context with no attached NSView is treated as not current, instead of
panicking. Closing a fullscreen child viewport can detach the view before the
next player repaint. Returning false lets glutin bind the surviving player's
view normally. The previous `expect("context to have a current view")` terminated
the whole application during that repaint.

Native regression: `cargo run --example visualizer_close_smoke`. This isolated
player/visualizer pair uses the production close handler without loading user
settings. Focus the visualizer, enter native fullscreen with its green window
button, and activate Close visualizer (Tab twice, Return). The player must remain
usable; reopen and repeat, then verify its native windowed close button too.
Diagnostic output is written to `/tmp/kaleido-fullscreen-close.log`.

Remove this override when an upstream version handles a missing context view
and the same native regression passes with that version.
