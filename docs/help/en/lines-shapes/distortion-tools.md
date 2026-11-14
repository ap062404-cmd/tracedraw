# Smoothing, smearing and roughening outlines

The shaping brushes of the Shape flyout reshape the outline of an object
where you drag over it, like a sculptor's tool: [[tool.smooth]],
[[tool.smear]], [[tool.twirl]], [[tool.attract_repel]], [[tool.smudge]] and
[[tool.roughen]].

## Using a shaping brush {#using}

1. Select the curves to change. Other objects are converted to curves first.
   The brushes change only the selected objects: with nothing selected a drag
   does nothing, and the status bar says to select an object first.
2. Choose the tool and set its **Nib size** (the radius of the brush, 10 mm
   at first) and its other settings on the property bar. The nib can also be
   sized on the drawing: hold Shift, press and drag across (right grows the
   circle, left shrinks it), or hold Shift and turn the mouse wheel (Alt and
   the wheel when the wheel is set to scroll instead of zoom).
3. Drag over the outline.

The effect is strongest at the center of the nib and fades to nothing at its
edge. Only the parts of the outline within reach change; open curves stay
open and closed ones stay closed. Each drag is one undo step. The settings
are kept for the next time.

## Smooth {#smooth}

The [[tool.smooth]] tool removes wobbles and extra nodes where you drag.
**Rate** (1 to 100%, 20% at first) sets how strongly.

## Smear {#smear}

The [[tool.smear]] tool (<kbd>W</kbd>) pulls the outline along with the
pointer, as if you smeared wet paint. **Pressure** (1 to 100%, 100% at first)
sets how far it moves.

## Twirl {#twirl}

The [[tool.twirl]] tool turns the outline around the pointer into a swirl.
**Speed** (1 to 100%, 50% at first) sets how fast it turns, and the direction
buttons choose counterclockwise or clockwise; hold <kbd>Alt</kbd> or
<kbd>Shift</kbd> while dragging to turn the other way. Holding the pointer
still keeps turning.

## Attract and Repel {#attract-repel}

The [[tool.attract_repel]] tool pulls the outline inside the nib towards the
pointer, or pushes it away, evenly in every direction. **Speed** (1 to 100%,
50% at first) sets how fast; choose Attract or Repel on the property bar, or
hold <kbd>Alt</kbd> or <kbd>Shift</kbd> while dragging to swap them. It works
from the moment you press: holding the pointer still keeps pulling or pushing
at a steady pace until the effect is complete, then the outline stays put
(Attract gathers it towards the middle of the nib, Repel pushes it out to at
most the edge of the nib).

## Smudge {#smudge}

The [[tool.smudge]] tool (<kbd>V</kbd>) carries the outline under the nib
along with the pointer: a stroke that crosses the outline draws it out into
a tongue about half as wide as the nib, whose sides run smoothly into the
rest of the outline, the same whether the stroke is quick or slow.
**Pressure** (1 to 100%, 100% at first) sets how far the outline follows.

## Roughen {#roughen}

The [[tool.roughen]] tool (<kbd>E</kbd>) adds jagged spikes along the outline.
**Frequency** (1 to 10 spikes per 10 mm, 5 at first) sets how many, and
**Spike size** (2 mm at first) how tall. The spikes are straight, so they stay
sharp. A stroke roughens each part of the outline once: going back over the
same place, or holding the button, adds no more spikes. Going over an outline
again in a new stroke makes it rougher; spikes that cross the outline stay
filled, with no white gaps between them.
