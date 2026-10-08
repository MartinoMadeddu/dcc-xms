# Edit Poly

One node that holds an ordered list of polygon operations, modelled on the Edit Poly modifier of 3ds Max. Add it from the add-node menu under Modify, and connect a mesh to its input.

While an Edit Poly node is selected, the viewport shows that node with a wireframe and the current selection, whatever the view flag says.

## Selection

Levels: Vertex, Edge, Border, Polygon, Element.

- Border selects open edges, a whole border at a time.
- Element selects polygons, a whole connected piece at a time.

Sources:

| Source | Selects |
|---|---|
| Picked | What you click or box-select in the viewport |
| All | Everything at the current level |
| By normal | Polygons facing within an angle of a direction |
| In box | Components inside an axis-aligned box |

In the viewport: click replaces the selection, Ctrl adds, Shift removes, dragging draws a box. Only the side facing the camera can be picked.

Modifiers: Grow, Shrink, Invert, Clear. At Edge level, Loop and Ring extend the selection.

A rule-based selection is stored as the rule, so the operation adapts when the incoming mesh changes. A picked selection is stored as component indices.

## Operations

Each operation uses the selection that was active when it was added.

| Operation | Level | What it does |
|---|---|---|
| Extrude | Polygon | Raises the selection. Group, local normal or by polygon |
| Bevel | Polygon | Extrude, then grow or shrink the outline |
| Inset | Polygon | New ring of polygons inside the selection |
| Bridge | Polygon, Edge, Border | Joins two openings or two groups of polygons with quads. Needs exactly two, with the same edge count |
| Flip | Polygon | Reverses the normals |
| Detach | Polygon | Makes the selection a separate element |
| Tessellate | Polygon | Splits each polygon into quads |
| Connect | Edge, Vertex | New edges across polygons, between selected edges (with segments) or two selected vertices |
| Remove | Edge, Vertex | Removes without leaving a hole |
| Cap | Any | Fills open borders. At Edge or Border level, only the selected ones |
| Weld | Any | Merges selected vertices closer than a threshold |
| Collapse | Any | Collapses each connected part of the selection to a point |
| Break | Any | Gives each polygon around the selected vertices its own copy |
| Delete | Any | Deletes the selection and the polygons that use it |
| Transform | Any | Move, rotate, scale about the selection centre. Falloff above zero is soft selection: vertices within that distance follow partly |
| Make planar | Any | Flattens across X, Y, Z or the best fitting plane |
| Relax | Any | Moves vertices towards the average of their neighbours |
| Subdivide | Whole mesh | Catmull-Clark subdivision |
| Chamfer | Vertex, Edge | Cuts the selected corners or edges back by an amount |
| Extrude vertex | Vertex | Raises each vertex into a spike with a base width |
| Extrude edge | Edge | Raises edges. Open edges grow new polygons |
| Outline | Polygon | Grows or shrinks the outline of the selection in its plane |
| Hinge | Polygon | Turns the selection about one of its edges, in segments |
| Slice | Any | Cuts new edges where an axis plane crosses the selection |
| Insert vertex | Edge | Splits each selected edge into segments |
| Triangulate | Polygon | Splits polygons into triangles |
| Turn | Edge | Turns the edge shared by two triangles |

In the list, each live operation has: an enable checkbox, a button to edit its selection (the viewport then shows the mesh entering that operation), move up, move down, collapse, remove.

## Manipulators

| Key | Tool |
|---|---|
| Q | Select |
| W | Move |
| E | Rotate |
| R | Scale |

The manipulator sits at the centre of the selection, on world axes. Move and Scale also have a centre handle: free move in the screen plane, and uniform scale.

Each drag is stored as a Transform operation. Further drags on the same selection update that operation. A scale after a rotation starts a new one.

The Delete key in the viewport adds a Delete operation.

## Collapsing

Two modes, chosen on the node:

- Keep live: operations stay editable. Collapse them one by one, or with Collapse all.
- Auto-collapse: adding an operation collapses the ones before it.

A collapsed operation is frozen and hidden from the list. It is still applied, in order, and still replays when the incoming mesh changes. A run of collapsed operations shows as one line with a Restore button. Collapsing a disabled operation removes it.

## Not done yet

Interactive cut and quickslice, extrude along spline, target weld, attach, edge split, MSmooth on a selection, preserve UVs, smoothing groups, material IDs, paint deformation, constraints, local manipulator axes. UVs do not pass through Edit Poly: unwrap after it.
