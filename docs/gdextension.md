# GDExtension findings (Godot 4.5.1)

Verified runtime behaviors of the pinned engine, learned by experiment. The
binding's unsafe contract (who frees what, which calls are legal when) lives in
the module doc of [gdext.rs](../profiler-core/src/engine/gdext.rs).

- **macOS trackpad scrolls never set wheel-button state.** Godot 4 delivers
  two-finger scrolls as `InputEventPanGesture`; only a physical wheel produces
  `MOUSE_BUTTON_WHEEL_UP/DOWN`, and that state lasts a single frame. A per-frame
  `Input.is_mouse_button_pressed` poll makes trackpads unscrollable and physical
  wheels timing-fragile, so scroll arrives from the C\# shim's `GuiInput` signal
  connection (same targeting as the `_gui_input` virtual, per stock 4.5.1
  `Control::_call_gui_input`).
- **Never call the engine about an engine-created `InputEvent`: every call shape
  tried froze the whole game** on the MegaDot fork (Mega Crit's custom 4.5.1
  with embedded CoreCLR; each freeze pinned by macOS `sample` plus disassembly).
  `Object.is_class` via `variant_call` inside the `_gui_input` virtual
  null-jumps in the fork's dispatch; the same call deferred to the per-frame
  refresh freezes identically, so the break is the call itself, not the callback
  context; the pure C `object_get_class_name` on the retained event parks the
  main thread inside the embedded CoreCLR's GC (stock 4.5.1 implements it with
  no managed code; the fork routes it into .NET and never returns). Calls about
  extension-created objects and global singletons are fine. Read the event in
  C\# (the game's own UI, `NScrollableContainer._GuiInput`, does exactly that)
  and forward plain scalars across the ABI.
- **The engine does not keep the theme font alive.** Store the
  `get_theme_default_font()` result Variant for the panel's lifetime and never
  destroy it: the object Ref inside keeps the Font alive, and a dropped font ref
  renders no text.
- **`Viewport.get_mouse_button_state` does not exist in the 4.5.1 API.** The
  failing call reads as "never pressed", which silently killed panel tab clicks
  while hover kept working. Mouse button state comes from the `Input`
  singleton's `is_mouse_button_pressed`.
