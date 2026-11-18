# Crossing settings: signal positions and rule templates

A crossing object keeps its own settings beside its `.sco`, in `<stem>.crossing.json`. They
are written by the crossing editor and read where they are needed; nothing is derived from
road arms, lane widths, driving side or kerbs, and the automatic junction path never reads
them to place anything by itself.

## The file

```json
{
  "version": 1,
  "copy": { "from": "Sceneryobjects\\...\\Kreuzung.sco" },
  "signals": [
    { "light": 0, "set": "", "x": 3.0, "up": 0.4, "fwd": -2.0, "heading": 123.7 }
  ],
  "rule_template": [
    { "path_index": 0, "kind": "bus", "value": 0, "extra": 0 }
  ]
}
```

- `copy` is there only when the crossing editor made the object a copy; its `from` is the
  file it was copied from. That is what makes later saves write in place, `.bak` and all.
- `signals` are the hand-placed signal heads: `light` is a zero-based index into the
  crossing's own traffic-light program, `set` an empty string for the map's default signal
  set, and `x`/`fwd`/`up` are in the crossing's frame (x right, fwd forward, up above its
  origin); `heading` is relative to the object's own heading.
- `rule_template` is the traffic rules a map object takes when the template is applied. It
  never holds a `[kill_rule]`.
- `signals`, `rule_template` and `copy` are left out when they say nothing; a file with
  nothing but `version` is not written, and one that is there is removed.

A file whose `version` is not the integer 1, or that is not JSON this version reads, is left
alone rather than written over. Keys this version does not know are kept. Old copies made
before the signals moved here hold only `{"from": "..."}`; that is read as `copy`.

## Where it is used

- **Crossing editor, Paths page.** `Signal positions` (below `Spline helpers`) and
  `Rule template` (below it). The template is set on every picked path at once - Ctrl+click
  picks more than one - and `bus` brings `no_cars` with it.
- **Object panel.** `Place signals as configured` for a crossing with a light program and
  positions in its settings; `Apply rule template` for an object whose settings hold rules.
- **Automatic junctions.** A junction writes the signal positions its arms need into the
  settings beside the plate (no head is put on the map), and a bus bay junction writes the
  bus-only rules of its own lanes there too.

## Windows acceptance checklist

These checks require the actual application. Automated or headless test success alone does
not mean they have been performed.

1. Open a crossing with its own traffic-light program in the crossing editor. On the Paths
   page expand **Signal positions** and **Rule template**.
2. Pick three paths (Ctrl+click), add **Buses only** in the template. Confirm six rows
   appear (three paths, `bus` and `no_cars`), then remove it and confirm all six go.
3. Pick one of the three and remove it: the row reads `some 2/3`. Add it again: `all`.
4. Move a path up and down, insert and delete one. Confirm the template follows the paths,
   and a deleted path's rules are gone.
5. Save. Confirm `<stem>.crossing.json` holds the template, the object's `.sco` is written
   as before, and a second crossing's settings are untouched.
6. With two objects of different `.sco`s selected, press **Apply rule template**. Confirm
   each gets its own template, its `[kill_rule]`s stay, other rules are replaced, and one
   Undo puts both back.
7. **Save as a copy** and confirm the new object's settings name the source in `copy.from`,
   hold the same signals and template, and the source's file and settings are unchanged.
8. Edit an old copy made before this change (only `{"from": ...}`): it is still a copy, and
   the next save writes the current form.
9. Open an automatic junction with traffic lights. Confirm it puts no signal head on the map
   and its settings hold one position per lit arm. Press **Place signals as configured** and
   confirm each head stands where the old generator put it (within 1 cm).
10. Rebuild that junction and confirm the settings are replaced with the new positions;
    delete the junction files and confirm its settings go with them.
11. Make a bus bay. Confirm the taper junctions keep no rules of their own, their settings
    hold `bus` and `no_cars` for the bay's lanes, and the road's own rules moved past the
    inserted lanes without gaining any.
12. Put an invalid entry in a settings file, and make the file unwritable in turn: confirm
    the log names the file and the reason, unrelated files are untouched, and a failed save
    reports failure and stays unsaved.

Release builds and the full test suites run on GitHub Actions only.
