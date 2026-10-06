# Viewport navigation

The menu at the top left of the viewport chooses the navigation style. The choice is kept between sessions. The box under the menu lists the keys of the current style.

| Style | Orbit | Pan | Zoom drag | Zoom in |
|---|---|---|---|---|
| Maya (default) | Alt + LMB | Alt + MMB | Alt + RMB, or Alt + LMB + MMB | Drag right |
| Houdini | Space + LMB | Space + MMB | Space + RMB | Drag right |
| XSI | S + RMB | S + LMB | S + MMB | Drag up |
| Blender | MMB | Shift + MMB | Ctrl + MMB | Drag up |
| Max | Alt + MMB | MMB | Ctrl + Alt + MMB | Drag up |
| Modo | Alt + LMB | Alt + Shift + LMB | Alt + Ctrl + LMB | Drag right |
| Unreal | Alt + LMB | Alt + MMB, reversed | Alt + RMB | Drag right |

In every style:

- The mouse wheel zooms while the cursor is over the viewport.
- F frames the mesh.
- Cmd or Super works in place of Alt.
- "Invert zoom drag" in the menu swaps the zoom direction.

Notes:

- Houdini: Alt works as well as Space. Space plays the timeline only on a tap, with no mouse button used while it was held.
- The zoom directions of the XSI and Modo styles were not confirmed against those packages.
- A camera drag has to start inside the viewport.
- Navigation follows the cursor position, not raw mouse motion, so it works through remote desktops and mouse sharing tools.
