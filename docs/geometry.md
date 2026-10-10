# Geometry

How geometry is held and passed between nodes, for anyone writing a node. Three rules keep Imago fast on production scenes and keep what it writes back to USD exact; they are at the end.

## `Geo`: attribute columns per context

`core::geo::Geo` is the geometry of packed primitives, of the nodes and of ICE: a topology and, for each context, named attributes.

| `Context` | values per | USD interpolation | ICE context |
|---|---|---|---|
| `Object` | one for the prim | `constant` | per object |
| `Point` | point | `vertex` | per point |
| `Primitive` | polygon or curve | `uniform` | per polygon |
| `Corner` | polygon corner | `faceVarying` | per sample |

- **Topology:** `Points`, `Mesh` (corner counts, point indices, orientation, subdivision scheme) or `Curves` (counts, basis, wrap).
- **Attributes:** a typed column (`Float`, `Int`, `Bool`, `Vec2`, `Vec3`, `Vec4`, `Quat`, `Mat4`, `Token`), optional indices (USD indexed primvars, where shared values are stored once), and a role (point, normal, vector, colour, texture coordinate) for writing the right USD type.
- **Names follow USD:** `points`, `normals`, `st`, `widths`, `displayColor`, and a primvar's own name for the rest. ICE calls the points `P`.
- `Geo::validate` checks that every attribute covers its context, every index is in range and the topology only uses points that exist.

Every column and every topology array is an `Arc`. Cloning a `Geo` copies pointers. Writing to a column goes through `Arc::make_mut`, which copies that column only if something else still holds it.

## Packed primitives

A `NamedMesh` is a packed primitive:

| Field | What it holds |
|---|---|
| `path` | Its prim path |
| `geo` | Its geometry, in its own space |
| `mesh` | A `MeshData` view of `geo` (`Geo::to_mesh`), for the code that still works on `MeshData`: the viewport, Edit Poly, the UV nodes |
| `place` | Where it is drawn: `InPlace` (the mesh is already there), `One(matrix)`, or `Many(matrices)` for a point instancer's copies. Prims from a stage are always placed: the matrix is their world transform with the stage's up-axis and unit correction |
| `source` | The primitive as it was loaded: geometry, placement, material, purpose |
| `stage` | The stage's hierarchy (`StageTree`), shared by every primitive of the stage |
| `material`, `look`, `purpose`, `picked` | As named |

Instanced primitives share one `geo` (and one `mesh`) between their copies.

## Edits

`crate::edits` finds what changed in a primitive by comparing it with its `source`, by pointer: an attribute still pointing at the column it was loaded with is unedited, however large it is. `edit_set` gives, for one stage, the changed primitives (placement, topology, attributes written or lost, material, purpose), the prims that no longer arrive, and the ones nodes made. `edits_between` does the same between two points of a network: the layer of the nodes in between.

Write USD (`usd_write`) turns an edit set into an override layer. A render delegate can use it to update only what changed.

## The three rules

1. **Replace only the columns you change.** Clone the `Geo`, write through `points_mut()` or `attr_mut()`, or `set` a new attribute. Never rebuild a whole `Geo` to change one thing: every attribute you rebuild becomes an edit, is copied in memory, and is written back to USD even though it did not change.
2. **Keep primitives placed.** Move a primitive by its `place`, not its points (`Placement::moved`). Work on geometry in its own space. For code that needs the mesh where it is drawn, `NamedMesh::world()` gives it, and `map_mesh` already takes one placed primitive there and back.
3. **Keep the primitive's identity.** Build changed primitives with `NamedMesh { geo, mesh, ..p.clone() }` so path, `source`, `stage`, material and purpose travel with it. If a node cannot keep the geometry traceable (it merges primitives, say), set `geo: None`: the edit is then written whole instead of as changed columns.

## Tests

`cargo test core::geo` (the model), `cargo test edits` (finding edits), `cargo test usd_write` (writing, and reading the written layer back through the composed reader).
