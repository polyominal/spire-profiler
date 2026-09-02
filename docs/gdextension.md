# GDExtension FFI (Godot 4.5.1)

Lessons from the hand-rolled GDExtension interop — read before touching
`profiler-core/src/engine/gdext.rs` or the panel's engine calls. The vendored
`vendor/gdextension_interface.h` (provenance banner included) is the
authoritative signature source for the pinned engine version.

The panel runs on a hand-rolled minimal GDExtension binding: the surface is two
Control subclasses, one `_draw` virtual, one `refresh` method, and a handful of
engine calls — far below the cost of a full binding crate.

- **Engine API is resolved by name at runtime.** `gdextension_entry` gets the
  engine's `get_proc_address` and resolves each interface function by its C name
  (`classdb_register_extension_class5`, `get_variant_from_type_constructor`,
  `variant_call`, ...). A missing symbol fails the entry loudly (it names the
  symbol); there is no compile-time-typed API to catch drift, so the vendored
  header is the reference.
- **Every engine method call is `variant_call`.** The ptrcall route needs
  `classdb_get_method_bind` signature hashes that are only published in the
  engine's `extension_api.json`; `variant_call` needs no such hashes, so it is
  the smaller, self-contained route. Values are built with
  `get_variant_from_type_constructor`, read back with
  `variant_get_ptr_internal_getter` (type-tag checked first — the internal
  getter is undefined behavior on a type mismatch), and every temporary Variant
  is `variant_destroy`ed on drop.
- **`_draw` dispatch is `get_virtual2`.** The class's `get_virtual_func` slot
  compares the incoming StringName against the cached virtual names by their
  interned data pointer (StringName is a single interned pointer in 4.5, so that
  is exact equality). Methods registered via
  `classdb_register_extension_class_method` never participate in virtual
  dispatch — `refresh` is a plain method, `_draw` is the only virtual.
- **macOS trackpad scrolling never sets wheel-button state.** Godot 4 delivers
  two-finger trackpad scrolls as `InputEventPanGesture` events; only a physical
  wheel produces `MOUSE_BUTTON_WHEEL_UP/DOWN`, and even then the button state
  lasts a single frame — a per-frame `Input.is_mouse_button_pressed` poll
  makes trackpads unscrollable and physical wheels timing-fragile. Scroll input
  therefore arrives from the C\# shim: it connects to each panel's `GuiInput`
  signal (same targeting as the `_gui_input` virtual, per stock 4.5.1
  `Control::_call_gui_input`) and forwards the raw event fields through the
  `spire_profiler_scroll_input` C export; the core translates and queues the
  pixels, and the per-frame `refresh` applies them.
- **The extension must NEVER touch an engine `InputEvent` object — every call
  shape tried froze the whole game** (on the MegaDot fork, Mega Crit's custom
  4.5.1 with embedded CoreCLR; each freeze pinned by macOS `sample` plus
  disassembly): `Object.is_class` via `variant_call` inside the `_gui_input`
  virtual → null-jump in the fork's dispatch; the same call deferred to the
  per-frame refresh → same freeze, proving the break is the call itself; the
  pure C `object_get_class_name` on the retained event → the main thread parks
  inside the embedded CoreCLR's GC (stock 4.5.1 implements it with no managed
  code — the fork routes this entry into the .NET runtime and never returns).
  The panels' own engine calls (draw\_rect/draw\_string/get\_position/
  queue\_redraw/Input singleton/...) are fine: they target objects the extension
  created or global singletons. The toxic surface is calls ABOUT engine-created
  input-event objects. When in doubt, read the event in C\# — the game's own
  UI (`NScrollableContainer._GuiInput`) does exactly that — and forward plain
  scalars across the ABI.
- **StringName is interned; String is refcounted — different ownership
  rules.** The fixed set of StringNames is created once at Scene init and leaked
  by design: StringName is a single interned pointer in 4.5, so the engine owns
  the deduped storage for the extension's lifetime. A String is a refcounted
  copy-on-write value, not an interned handle: the temporary String built for a
  Variant must be destroyed once the Variant has copied from it, or the
  engine-side buffer leaks.
- **Theme font lifetime is the critical hazard.** Fetch
  `get_theme_default_font()` lazily on the first draw and store the result
  Variant in the panel state, NEVER destroying it — the object Ref inside
  keeps the Font alive. A dropped font ref renders no text; a failed fetch
  disables text but not bars, with a one-shot warning.
- **Verify method names against the extension API before wiring a call.**
  `Viewport.get_mouse_button_state` does not exist in the 4.5.1 API; the failing
  call reads as "never pressed", which silently killed panel tab clicks while
  hover kept working. `xtask check-abi` verifies shim↔Rust exports, but
  nothing verifies Rust→engine method names — grep
  `extension_api_4.5.1.json` first. Mouse button state comes from the **`Input`
  singleton** (`global_get_singleton("Input")` + `is_mouse_button_pressed(1)`).
- **Panel parent class choice**: a `PanelContainer` parent paints its stylebox
  in `NOTIFICATION_DRAW`, which runs AFTER virtual `_draw` and would cover
  custom drawing — the panel is a plain `Control` and draws its own
  background/border.
- **Headless + registration**: both panel classes register at Scene init
  (`minimum_initialization_level = Scene`); the `panel class registered` line is
  a headless gate marker. The `_draw` virtual fires under the headless dummy
  renderer too (the boot logs `chart _draw active: N cmds`), so headless covers
  draw dispatch but not visual output — fonts/colors still need a real-play
  check.
- **Keep `#![deny(unsafe_code)]` intact**: the raw FFI (engine function
  pointers, raw `*mut c_void` reads, `extern "C"` callbacks) is quarantined in
  `engine/gdext.rs`; engine.rs scopes that module with `#[allow(unsafe_code)]`
  (the third such relaxation, after `abi` and `registration`). Every C callback
  routes through a panic-containment helper (mirroring `abi::contain`) so a Rust
  panic can never unwind into the engine.
- **Engine-free testing**: the interaction decision logic is factored into pure
  functions (`press_zone`, `dismiss_on_outside_press`, `content_signature`, the
  scrollbar state machine) that take the local `math::Vector2`/`Rect2` (plain
  `f32` structs — no engine calls), so they are unit-tested under nextest
  without booting Godot.
