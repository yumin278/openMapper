# Floating Assets: Windows acceptance checklist

These are manual acceptance steps, not a record of verified UI behaviour. Automated
layout tests do not replace running the editor on Windows.

- [ ] Open Assets with **B**. With no saved layout it is floating inside the main window.
- [ ] Drag its title bar. The tool and properties panels remain in place.
- [ ] Resize from each edge, including shrinking to the minimum size. Folder/grid content
      follows the available space; the window does not grow by itself over subsequent frames.
- [ ] Select an asset, resize again, and scroll the folder tree, thumbnails and details.
- [ ] Collapse and expand the title bar. The expanded dimensions are retained.
- [ ] Move the collapsed window, then expand it; its expanded size is still retained.
- [ ] Close using the title-bar close button, then press **B** to reopen; repeat using **B**
      for both closing and reopening.
- [ ] Use **Dock** to move Assets to the bottom, resize the docked panel, then use **Float**.
      Repeat the transition; both presentations retain search/filter/selection state.
- [ ] Move and resize the floating window, release the mouse, quit, then restart. Verify its
      position, expanded dimensions and floating/docked choice are restored.
- [ ] Restart with a smaller main window (or move to a smaller monitor). Verify the restored
      Assets window is constrained to the visible main-window area.
- [ ] With Assets over the map, move the mouse across its title bar, body, empty space,
      borders and collapsed title bar. The map cursor/highlight must not follow it.
- [ ] Click within Assets and turn the wheel over each scrollable region and empty space.
      No selection/placement should occur in the map, and the camera must not zoom.
- [ ] Type text containing **n**, **b** and other shortcuts into Search. It must only edit
      the search text. Move keyboard focus out of the text field and verify **N**/**B**
      work normally.
- [ ] Single-click an object thumbnail, leave Search unfocused, and use **N** to place that
      object in the map as before. Thumbnail drag-to-place is outside this acceptance scope.
- [ ] While dragging/resizing, verify `~/.openomsi/openmapper.json` (on Windows, under the
      user profile's `.openomsi`) is not rewritten on every frame. After releasing the
      mouse, verify the changed layout is saved; idle frames should not rewrite it.
- [ ] Repeat key operations in English, Russian and the preferred translated UI language.
