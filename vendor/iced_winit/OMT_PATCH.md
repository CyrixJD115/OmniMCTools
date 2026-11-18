# OMT patch to iced_winit 0.14.1

Vendored (MIT) for one addition: **pointer lock + raw mouse motion**, which
the OmniMCTools 3D viewer needs for game-style mouse-look and iced 0.14 does
not expose.

1. `src/omt_pointer.rs` (new): `request(lock)`, `locked()`, `take_motion()`.
2. `src/lib.rs`:
   - `pub mod omt_pointer;`
   - `ApplicationHandler::device_event` forwards `DeviceEvent::MouseMotion`
     deltas to `omt_pointer` (accumulated only while locked).
   - At the top of `Event::EventLoopAwakened` handling in `run_instance`,
     pending lock requests are applied to every window
     (`CursorGrabMode::Locked`, else `Confined`; cursor hidden while held).

3. `#![allow(deprecated)]` (upstream calls a deprecated futures method).

Nothing else changed. Drop the vendored copy if a future iced release
offers pointer lock / raw motion.
