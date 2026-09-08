# tool
*a feature in the tools layer extends the runtime's `describe`*

parent: base
layer: tools

> (suite) 2026-09-08T10:03:00
Plan item 9: a higher layer redefines a lower layer's function and calls `existing`; control flows up, names do not.

## overview
`tool` sits above the runtime in `layers.md` and adds 1000 to `describe`; the runtime never names it.

## interface
- `described by the tool` gives most's 20 plus 1000.

## rules
- A feature names only its own layer and the layers below it; a redefinition comes from a layer at or above the earlier definition's.

## testing
>described by the tool() → 1020

## hostile
`described by the tool()` called from a runtime feature is refused: "'described by the tool' belongs to feature tool, whose layer is above base's: a name reaches down or sideways, never up". A feature with `parent: nobody` is refused: "parent 'nobody' is not a feature of the store". A `layer: kernel` not in `layers.md` is refused: "layer 'kernel' is not in layers.md (runtime, tools)". A runtime feature whose parent is `tool` is refused: "a feature's layer is at or above its parent's". A root feature with no `layer:` line is refused: "has no layer and no parent to take one from".
