# Scripts panel

A script is JavaScript written against an object model of the drawing:
documents, pages, layers, shapes, their fills, outlines, curves and nodes.
Every change a script makes goes through the same commands the interface
builds, so a whole run is one undo step, named `Script` in the history.

Tools > Scripts opens the panel: a code editor with a line margin, the
output of the last run, **Run**, **Debug** (see `behavior/script-debugger.md`),
**Record**, **Load...**, **Save...** and **Install as Filter** (see
`behavior/script-filters.md`). Recorded macros are plain scripts.

A **JavaScript / VBA** pair of buttons above the editor picks the language.
JavaScript is the object model described here. VBA writes the snippet as a
macro written for a drawing suite, which is compiled and run over the macro
object model instead (see `behavior/vba.md`); the desktop only.

## Units and conventions

- Lengths in millimetres, page coordinates: the origin is the bottom left
  corner of the page, x grows to the right and y upwards.
- Angles in degrees, counter-clockwise.
- Text sizes in points.
- Items are counted from 1, as the macro object model counts them:
  `Pages.Item(1)`, `Pages.Item(n).Layers.Item(1)`, `Shapes.Item(1)`,
  `Nodes.Item(1)`, `SubPaths.Item(1)`, `Effects.Item(1)`.
- `Shape.PositionX` and `Shape.PositionY` are the bottom left corner of the
  object's bounding box; `CenterX` and `CenterY` are its centre.
- `Shape.SetOpacity(pct)` takes a transparency: 0 is opaque, 100 invisible.
  `Shape.Transparency` reads it back in the same numbers.

## Running a script

**Run** evaluates the panel's script. What it prints with `print(text)` (or
`console.log`) goes to the panel's output line by line. When the script
prints nothing, the value of its last expression is shown instead, so a
one-liner such as `ActivePage.Shapes.Count` answers with a number.

An error stops the script and is shown with its message. The changes made
before the error are kept, as one undo step.

`MsgBox(text)` prints its text to the output; there is no dialog.

## A first script

A fan of twelve rounded bars, each a little more orange than the one
before. This is the example in `README.md`, word for word, and
`scripting::tests::the_readme_example_draws_a_fan_of_bars` runs it.

```js
for (let i = 0; i < 12; i++) {
  const bar = ActiveLayer.CreateRectangle(80, 120, 60, 12, 3);
  bar.Fill.ApplyUniformFill(Color.RGB(255, 60 + i * 15, 60));
  bar.Rotate(i * 15);
}
```

Reading a drawing works the same way: `ActivePage.Shapes.All` is a
`ShapeRange` of everything on the page, and each object answers with its
own properties.

```js
let widest = 0;
ActivePage.Shapes.All.forEach(s => {
  if (s.SizeWidth > widest) widest = s.SizeWidth;
});
print(`${ActivePage.Shapes.Count} objects, the widest ${widest.toFixed(2)} mm`);
```

## The object model

The model is built in JavaScript on top of one native call,
`__td(method, arguments)`, which the page does not need to use. Each method
below names the call it makes.

### Application and the active document

| Name | Meaning |
|---|---|
| `Application.ActiveDocument` | the drawing in the window |
| `Application.ActivePage` | its current page |
| `Application.ActiveLayer` | the current layer of that page |
| `Application.ActiveSelection` | the selected objects |
| `Application.ActiveShape` | the first selected object, or `null` |
| `Application.Version` | the program's version, as text |
| `Application.Name` | `'TraceDraw'` |
| `Application.CreateColor` | the `Color` class |

`ActiveDocument`, `ActivePage`, `ActiveLayer` and `ActiveSelection` are also
global, so a script can start with either form.

### Document

| Name | Meaning |
|---|---|
| `Name` | the drawing's title |
| `ActivePage`, `ActiveLayer` | the current page and its current layer |
| `Pages` | the document's pages |
| `Selection`, `SelectionRange` | the selected objects as a `ShapeRange` |
| `SelectAll()` | selects every object of the current page |
| `ClearSelection()` | selects nothing |
| `AddPages(n)` | adds `n` pages (1 when omitted) and returns the last |
| `CreateRectangle(x, y, w, h, radius)` | on the current layer |
| `CreateEllipse(x, y, w, h)` | on the current layer |
| `CreateArtisticText(x, y, text, font, sizePt)` | on the current layer |
| `CreatePath(pathData)` | on the current layer |
| `Export(path, format)` | writes the drawing to a file; the file name's extension picks the format (`svg`, `pdf`, `eps`, `ps`, `prn`, `dxf`, `emf`, `wmf`, `plt`, `psd`, `html`, `png`, `tdraw`), and `format` is accepted but not used |

`BeginCommandGroup(name)` and `EndCommandGroup()` are accepted and do
nothing: a script's changes are one undo step already.

### Pages, layers

`Pages` has `Count`, `Item(i)` and `forEach(f)`.

`Page` has `Name`, `SizeWidth`, `SizeHeight`, `Index`, `Layers`,
`ActiveLayer`, `Shapes`, `SetSize(w, h)`, `CreateLayer(name)` (returns the
new layer) and `Activate()`.

`Page.Shapes` is narrower than a layer's: it offers `All` (a `ShapeRange`
of every object on the page) and `Count`.

`Layers` has `Count`, `Item(i)`, `forEach(f)` and `Find(name)` (a `Layer`,
or `null`).

`Layer` has `Name` (read and set), `Visible` (read and set), `Shapes`,
`Activate()`, and the create methods:

| Method | Object |
|---|---|
| `CreateRectangle(x, y, w, h, radius)` | rectangle, `radius` 0 for square corners |
| `CreateEllipse(x, y, w, h)` | ellipse in that bounding box |
| `CreatePolygon(x, y, w, h, points)` | regular polygon |
| `CreateStar(x, y, w, h, points, sharpness)` | star, `sharpness` 0 to 1 |
| `CreateArtisticText(x, y, text, font, sizePt)` | text at the baseline start |
| `CreateParagraphText(x, y, w, h, text, font, sizePt)` | text in a frame |
| `CreateLineSegment(x1, y1, x2, y2)` | straight line |
| `CreateCurve(points, closed)` | straight segments through `[[x, y], ...]` |
| `CreatePath(pathData)` | SVG path data, `M L H V C S Q T A Z` |
| `ImportBitmap(path)` | picture from a file |

Every create method returns a `Shape`. New objects go on the layer in front
of the others, with no fill and a thin black outline; text is black with no
outline.

### Shapes and ranges

`Shapes` (a layer's) has `Count`, `Item(i)`, `All` (a `ShapeRange`),
`FindShape(name)` (a `Shape`, or `null`) and `forEach(f)`.

`ShapeRange` is a set of objects, made by `new ShapeRange([id, id, ...])`:

| Name | Meaning |
|---|---|
| `Count`, `Item(i)`, `Shapes`, `forEach(f)` | the objects in it |
| `Delete()`, `Move(dx, dy)` | on all of them |
| `Group()` | groups them, returns the group as a `Shape` |
| `CreateSelection()` | selects them |
| `AlignLeft()`, `AlignRight()`, `AlignTop()`, `AlignBottom()` | to the last object in the range |
| `AlignCentersHorizontally()`, `AlignCentersVertically()` | centres on the last object |

### Shape

| Name | Meaning |
|---|---|
| `Name`, `Text` | read and set |
| `Type` | `Rectangle`, `Ellipse`, `Polygon`, `Star`, `ComplexStar`, `Curve`, `Text`, `Group`, `Bitmap`, `ClipFrame`, `Table`, `Symbol` |
| `PositionX`, `PositionY` | bottom left of the bounding box |
| `SizeWidth`, `SizeHeight` | the bounding box's size |
| `CenterX`, `CenterY` | its centre |
| `Transparency` | from 0 (opaque) to 100 |
| `PathData` | the outline as SVG path data, in page millimetres |
| `Fill`, `Outline` | see below |
| `Curve`, `Effects` | see below |
| `SetPosition(x, y)` | places the bounding box's bottom left corner |
| `SetSize(w, h)` | resizes the bounding box |
| `Move(dx, dy)`, `Rotate(deg)`, `Stretch(sx, sy)`, `Skew(ax, ay)` | transforms; `Rotate` turns about the centre |
| `Duplicate(dx, dy)` | returns the copy |
| `Delete()` | removes the object |
| `ConvertToCurves()` | returns the same object, now a curve |
| `OrderToFront()`, `OrderToBack()` | in its layer |
| `SetOpacity(pct)` | transparency, 0 opaque to 100 invisible |
| `AddDropShadow(dx, dy, opacity)` | a drop shadow, `opacity` in percent (50 when omitted) |
| `CreateSelection()`, `AddToSelection()` | selects it, or adds it to the selection |

### Fill, outline, colour

`Color.RGB(r, g, b)` takes 0 to 255, `Color.CMYK(c, m, y, k)` and
`Color.Gray(v)` take 0 to 100 (0 black, 100 white). `Color.Hex` reads a
colour as `"#rrggbb"`.

`Shape.Fill` has `Type`, `UniformColor`, `ApplyUniformFill(color)`,
`ApplyFountainFill(from, to, angle)` and `ApplyNoFill()`.

`Shape.Outline` has `Width`, `Color` (read and set),
`SetProperties(widthMm, color)` and `SetNoOutline()`.

### Curves and nodes

`Shape.Curve` has `Closed`, `SubPaths` and `Nodes`.

`SubPaths` has `Count`, `Item(i)` and `forEach(f)`; a `SubPath` has `Closed`,
`Index` and `Nodes`.

`Nodes` has `Count`, `Item(i)`, `First`, `Last` and `forEach(f)`; a `Node`
has `PositionX`, `PositionY`, `SubPathIndex`, `IsEnding`, and its `list` can
be narrowed to one subpath.

`Node.Type` reads and sets `cusp`, `smooth` or `symmetrical`. A node can be
moved with `SetPosition(x, y)` or `Move(dx, dy)`, taken out with `Delete()`,
and one added before it with `AddNodeBefore(t)`, `t` being where along the
segment to put it (0.5 when omitted).

A curve's `Nodes` lists every node of the object in path order, whichever
subpath each belongs to; `Node.SubPathIndex` says which, counting from 1.

### Effects

`Shape.Effects` has `Count`, `types`, `Item(i)`, `Add(effect)`,
`Remove(i)`, `Clear()`, `AddExtrude(dx, dy)` and
`AddBlockShadow(dx, dy, color)`.

`Add` takes the effect as it is written in a `.tdraw` file, so any effect the
program has can be added by passing its object. The effects that are live
effects counted here are the ones the Effects docker lists, and an object's
drop shadow is not one of them: `Shape.AddDropShadow(dx, dy, opacity)` sets
the object's shadow, which `document_info` reports apart from `effects`.
So `AddDropShadow` leaves `Effects.Count` as it was.

## Limits

A script cannot remove or rename a page or a layer, or ungroup a group:
the commands exist in the engine, but the object model has no method for
them. A script also cannot undo or redo; the history belongs to the
interface.

Scripts an AI service writes (AI Drawing, and the check when a script filter
is installed) run with limits instead: no file access, a bounded number of
loop iterations, calls and changes, and nothing kept unless all of it runs.
Those limits are in `scripting.rs`, and `behavior/ai-drawing.md` describes
what the user sees.

## Checks

- `scripting::tests::script_creates_and_moves_a_rectangle`
- `scripting::tests::the_readme_example_draws_a_fan_of_bars`
- `scripting::tests::script_error_is_reported`
- `scripting::tests::curves_show_their_subpaths_and_nodes`
- `scripting::tests::effects_are_listed_and_removed`
- `scripting::tests::selection_round_trip`
- `ai::tests::the_reference_names_only_methods_the_object_model_has` holds
  the description in `ai.rs` to the model
