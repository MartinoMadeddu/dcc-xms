# Undo and the History pane

![The History pane: a moved node, a Bevel changed, a bypass, and two undone steps kept as a branch](xms_history.png)

## What is undone

Everything in the scene: nodes, their parameters, Edit Poly operations, names, bypass, wires, where nodes sit in the graph, and which node shows in the viewport. That is what a saved graph holds.

The interface is not undone: the camera, the layout, the theme, the playhead, panning and zooming the graph, and which node is selected never make a step. Undoing does select the node the step changed, so the change is in front of you.

| To | Do this |
|---|---|
| Undo | Ctrl+Z (Cmd+Z on macOS), or Undo in the History pane |
| Redo | Ctrl+Shift+Z or Ctrl+Y, or Redo in the History pane |
| Go to any step | Click it in the History pane |
| Go back to undone steps after a new change | Click the branch row under the step where they leave the list |
| Keep a step whatever the memory | Right-click it, "Pin: keep this step" |
| See only what changed one node | Select the node, tick "Selected node" |

While a text field has the keyboard, Ctrl+Z undoes typing in that field, not a step.

## Steps

A step is one gesture, not one frame. Dragging a slider, a node or a manipulator makes one step when the mouse button is let go, however many frames it took. Typing in a field makes one step when the field is left.

Every step is named from what changed: "Add Cube", "Connect Cube to SeaMine", "SeaMine: #6 Bevel height 0.12 to 0.29", "SeaMine: add Extrude", "SeaMine: restore 5 operations", "Bypass Cube", "Move 3 nodes". Loading a template or opening a graph is a step too, named after it, so it can be undone.

Picking components for Edit Poly is not a step of its own. The pick goes into the operation that uses it: pick 12 polygons and extrude them, and that is one step, "SeaMine: add Extrude". Undoing it takes away the extrude and the pick.

## Branches

Undoing and then changing something does not throw the undone steps away. They stay as a branch, shown as one row under the step where they leave the list ("2 steps on another branch, last: ..."). Click it to go to the end of that branch. Redo then follows the branch last visited.

## How it works

Each step is a snapshot of the scene. Nodes that did not change are shared with the step before, so a step costs only what changed: a parameter change on one node of a large graph takes about the size of that node. Undo puts an earlier snapshot back. Nothing has to know how to reverse itself, so a new node type is undoable without any work. The results cooked for the earlier state are usually still in the caches, so going back is quick.

Every frame the scene is compared with the current step. Any change that is not part of a gesture in progress becomes a step, whatever made it, so nothing can get past the history.

The steps may take 256 MB together. Past that, the oldest go first: side branches before the main line. The current step and pinned steps always stay. Node ids are never given out twice, so a node made after an undo never takes the id of one that was undone.

## Not done yet

- Changes inside an ICE subnet are not in the history.
- The history is not saved with the graph. It starts empty each time the program starts.
- Hovering a step does not preview it.
