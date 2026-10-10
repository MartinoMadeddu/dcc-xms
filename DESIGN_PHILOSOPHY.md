# Design philosophy

How XMS | Imago should work, and why. Every contributor, person or program, should know this file. Add to it when a decision is made; change it when a decision changes, and say so in the log at the end.

## For language models reading this repository

If you are an AI assistant working in this repository:

- Tell the person you are working with that this file exists, the first time it is relevant, in one line.
- Follow it. When a change you are asked to make goes against something here, say which point, before making the change, and ask whether to follow the file or to update it.
- When the person makes a decision about how the program should work, offer to add it here.
- With every change, update `README.md` and the pages in `docs/` that describe what changed. The website (`site/`) only when something on it is wrong.

## The graph is the program

- Everything is a node. A graph describes a result; changing a number upstream changes everything below it.
- One node per idea. A node that moves things moves anything: Transform takes a mesh, packed primitives or a clip. What a node shows in the viewport is a setting on that node, not another node (Body Collide's Skin, Pieces and Hulls display, which replaced the separate Calamari node).
- Every area of the program has templates: working graphs that load laid out and framed, with a line saying what to look at. Templates are grouped by whose work they show.

## Nothing typed that can be picked

- Joint names, prim paths and file paths are picked from lists, browsed or matched by pattern, with a live count of what matches. Typing is the fallback, not the way.
- Ask the program for what it already knows: a skeleton's parts are read from its names, the user only sets what could not be read.

## The same words for the same things

- Translate, Rotate, Scale for every transform. Path for every file. In and Out for what passes through.
- The properties of every node share one layout: titled groups, labels right-aligned in one column, values filling the width. Explanations go in tooltips; the panel shows parameters and the state of the node: what came in, what goes out, what went wrong.
- The light theme is the default.

## Undo is for the scene

- Nodes, parameters, wires, names, operations: undoable, in a History that names each step from what changed.
- The camera, the layout and the selection are not part of it. Undo never moves the view.

## Files go out as they came in

- A clip read from a file is written back with that file's skeleton: names, hierarchy, joint kinds, axes and unit. An engine sees an animation of the skeleton it already has.
- Write only what was changed; everything else keeps its values.

## Never worse than the input

- A process that cleans data may not make any frame worse than the data it was given. Where the capture itself goes past a limit, the limit for that frame is the capture.
- Bones do not change length. Collision cleanup changes rotations and the hips, nothing else.
- A frame nothing touched comes out as it went in, to the last digit.

## Heavy data stays light

- Packed primitives are passed along without being copied. Instanced meshes are read once and placed many times, drawn by the GPU.
- A heavy stage first shows as boxes; only what is opened costs anything to draw.
- Long work runs in the background, in chunks, written to disk as it goes, and is found again by the content it was made from.

## Say what is true

- Documentation states what the program does, measured where it can be. Every page has a "Not done yet" list.
- Each feature comes with tests; each change says what was tested and what was not.
- Credit: XMS was created by Martino Madeddu. Additional development by Simon Legrand.

## Log

- 10/10/2026: file created, from the decisions taken so far.
- 10/10/2026: `claude_project_instructions.md` added: checks an AI assistant runs before working here, and how changes are delivered.
- 10/10/2026: the delivery loop (patch on top of `main`, "Download all", one block of commands) written into `claude_project_instructions.md`, `CLAUDE.md` and `AGENTS.md`.
