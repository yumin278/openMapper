# Manual crossing signal positions

This feature uses only positions entered by the user. It does not derive positions from
road arms, lane widths, driving side or kerbs. The automatic junction generation path
remains separate.

## Configuration

The active map stores positions in `openMapper/crossing_signals.json`:

```json
{
  "version": 1,
  "crossings": {
    "sceneryobjects\\example\\crossing.sco": [
      {"light": 0, "set": "", "x": 3.0, "up": 0.4, "fwd": -2.0, "heading": 123.7}
    ]
  }
}
```

Coordinates are local to the crossing: x right, fwd forward, up above its origin.
`light` is a zero-based index into its traffic-light program. An empty `set` uses the
map's signal-set default. The map-level file applies to all instances of that `.sco`;
it is not per map-object ID.

Placement follows the configured yaw-only transform: local x/fwd rotate by the map
object's heading; up is added to its origin world z, independently of terrain under each
lamp. It does not infer pitch/bank compensation or road-edge placement.

## Windows acceptance checklist

These checks require the actual application. Automated or headless test success alone
does not mean they have been performed.

1. Open an original crossing with its own traffic-light program in the crossing editor.
   On the Paths page expand **Signal positions**, below **Spline helpers**.
2. Add a point using **A signal** and one using **At the picked end**. Confirm the latter
   takes the path's end position and heading, and both sample the local model surface
   for their initial up value. No point should appear without a user action.
3. Edit light, set ID, x/f/h and heading. Check unnamed light programs show `(no name)`.
   Verify the coloured circles, four-metre heading lines and light labels in 2D. Frame
   the view after moving a point well outside the original crossing.
4. Change only a signal position, Save, close and reopen. Confirm the JSON contains the
   change even when the `.sco` text itself did not change. Confirm a second crossing's
   entry is unchanged.
5. Exercise Undo/Redo, Revert and the unsaved-close dialog on position-only edits.
6. Save as a new object file. Confirm positions are stored under the new `.sco` key and
   the source entry is preserved.
7. Select a crossing instance on the map and use the independent **Place signals as
   configured** section. Verify this works with original and persisted crossings, not
   only openMapper-generated `junction_` objects.
8. Repeat placement. Confirm there is only one configured set of heads/poles, and Undo
   restores the previous children as one operation. Placement replaces all records
   whose `var_parent` is this crossing, not only recognised lamp models.
9. Repeat at a non-right-angle rotation and on steep terrain, for both absolute-height
   and terrain-relative crossings. The head world height must be crossing origin world
   height + configured up + signal-set head height, within 1 cm. Test signal heads and
   poles with different height modes as well.
10. Save the map and reopen it. Confirm signal extra values and `var_parent` links remain.
    Check the log for local/world coordinates, height modes and accurate placement/skip
    counts.
11. Generate an ordinary automatic junction with physical signals enabled. Confirm its
    original signal placement is unchanged; no table or manual points are required.
12. Try a malformed configuration and an unwritable destination. Confirm diagnostics
    identify the file and cause, no unrelated configuration is replaced, and failed
    saves do not report success or clear unsaved state.

Release builds and full test suites run on GitHub Actions only. Before pushing, run the
permitted local debug check, filtered tests and translation check.
