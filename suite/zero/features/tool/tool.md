# tool
*a feature in the tools layer extends the runtime's `describe`*

parent: base
layer: tools

> (suite) 2026-09-08T10:03:00
Plan item 9: a higher layer redefines a lower layer's function and calls `existing`; control flows up, names do not.

## overview
`tool` sits above the runtime in `order.md` and adds 1000 to `describe`; the runtime never names it.

## interface
- `described by the tool` gives most's 20 plus 1000.

## rules
- A feature names only its own layer and the layers below it; a redefinition comes from a layer at or above the earlier definition's.

## testing
>described by the tool() → 1020

## hostile
`described by the tool()` called from a runtime feature is refused: "'described by the tool' belongs to feature tool, whose layer is above base's: a name reaches down or sideways, never up". A feature with `parent: nobody` is refused: "parent 'nobody' is not a feature of the store". A `layer: kernel` not in `order.md` is refused: "layer 'kernel' is not in order.md (runtime, tools)". An `order.md` without the words `lowest first` before its list is refused: "order.md orders the layers, lowest first: say so on a line before the list"; a store still holding `layers.md` is told "the layer file is order.md now". Two features whose origins share a timestamp are refused, "features base and more share the origin 2026-09-08T10:00:00: composition order is the origin's time, so give one a later origin, or write `same commit` after both timestamps to order them by name"; with `same commit` on both they compose by name. A runtime feature whose parent is `tool` is refused: "a feature's layer is at or above its parent's". A root feature with no `layer:` line is refused: "has no layer and no parent to take one from".
