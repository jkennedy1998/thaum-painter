# windows first-test findings (2026-09-06)

First real Windows run of the gnu-targeted build (`orchestration/builds/windows/thaum painter/`,
sent to J as `thaum-painter-windows.zip`). App launches and works. Three issues below, in J's
priority order of discovery — 1 and 2 fixed 2026-09-06 (renderer `81eb5ed`, painter `15ad8d2`); 3 open.

## 1. Huion tablet pen: position works, clicks do not

Symptom: pen position is tracked over the canvas, but pen-down never draws/presses. Nothing
in the log — the events never reach the app.

Diagnosis: the window/input seam (`thaum-renderer/tools/window-surface/`) only consumes
`WindowEvent::CursorMoved` / `MouseInput` from winit 0.30. On Windows, Huion tablets run
through the Windows Ink / WM_POINTER stack; winit's pen handling surfaces pen movement but
does not reliably deliver pen-down as `MouseInput`. Mouse still works, which matches what J saw.

Direction (standardized, not vendor-driver chasing): treat Windows Ink (WM_POINTER) as the
supported tablet path — the same API Photoshop standardized on. Concretely: handle pen
pointer events with pressure at the window-surface seam (likely a small `windows-rs`
WM_POINTER handler alongside winit), rather than trying to support Huion/XP-Pen vendor
driver quirks individually. Pressure is wanted for a painter eventually anyway, so the seam
should carry `pressure` from day one.

## 2. Save As → cancel: looks like a hard crash on Windows

Symptom: Save As, then cancel in the dialog → program appears frozen; pressing Enter in the
console log window "uncrashes" it, then the app closes itself.

Diagnosis (confirmed in code): on dialog cancel, `save_file()` returns `None`, and
`prompt_save_document_root` (orchestration/build-commands/src/main.rs) falls through to
`prompt_path_in_terminal` — the jobo Linux portal-fallback that blocks reading a path from
stdin. On Windows that reads the hidden console: the app looks hung, Enter returns empty →
`None`, and the session flow after that exits. This is a Linux-specific workaround leaking
onto Windows, not a real crash.

Fix shape: gate the terminal fallback to Linux (or to "stdin is a usable TTY"), and make
dialog cancel simply cancel the save. Cancel must never route into any prompt.

## 3. Default module layout is bad

Symptom: first-run default placement of modules is "terrible" per J.

Direction: override the module-registry defaults so the out-of-box layout is sensible;
defaults live with the module registry / layout seams, not in saved documents. Design of the
good default layout still needed from J (or a first pass for J to critique).
