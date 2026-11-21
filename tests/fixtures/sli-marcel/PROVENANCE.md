# Provenance of the Marcel spline fixtures

These two files are regression fixtures for the Road Maker SLI import. They are
not roads made by openMapper: they reproduce the cross-section of two splines
from OMSI 2 so the import can be checked against real content instead of a shape
the tests invented.

| Fixture | Source (GitHub) | Git blob SHA-1 | Bytes | SHA-256 of the fetched file |
| --- | --- | --- | ---: | --- |
| `Hstr_6spur_Wilhelm2.sli` | <https://github.com/smrtosg/Hillgrove-Woodlands-Town/blob/main/OMSI%202/Splines/Marcel/Hstr_6spur_Wilhelm2.sli> | `5e012d2af0d8991247f59e1b1f35a49c39d8d97c` | 1395 | `3326e11cfa5e513e7f5efe42b50f4262020363440f3d1b892f90763171b447a5` |
| `str_4spur_zeppelin1.sli` | <https://github.com/smrtosg/Hillgrove-Woodlands-Town/blob/main/OMSI%202/Splines/Marcel/str_4spur_zeppelin1.sli> | `4670d76c8635755a4fa1ce64b18e89246468937e` | 3479 | `652d674faf007f57eff33df569052705199d890b7ba1ff4bd4dafdbed1dc58a3` |

Both were fetched on 2026-11-22 through the GitHub REST API. The local OMSI 2
installation also has these splines under `Splines\Marcel\`; its copies are 1586
and 3819 bytes (dated 2015-03-14) because they still carry the comments and CR LF
line endings, and their keyword blocks are identical to the fetched files'.

The full raw files are not copied into this repository; they stay at the links
above. The fixtures contain only the keyword blocks.

## What was kept

Every keyword block, in the original order, with the original numeric text:

- `[length]`, `[halfcantwidth]` (Zeppelin1 only)
- all `[heightprofile]` blocks
- all `[texture]` blocks
- all `[profile]` blocks and their `[profilepnt]`s
- all `[path]` blocks

Nothing numeric was rounded, reordered or recalculated. Free-text comments and
section headings (`File created with StreetCreator`, the German explanations,
the tab-indented `[path]` example in Zeppelin1) were removed, and CR LF line
endings became LF. Removing the comments does not change what OMSI or
`omsi-scenery` parses: a keyword line must start with `[`, and the removed
example is indented with tabs.

## How to check it

The extraction is mechanical: read the file, keep each line that is exactly
`[keyword]` at the start of a line, then keep the next non-blank lines as the
block's values (`heightprofile` 4, `profile` 1, `profilepnt` 4, `texture` 1,
`path` 5, `length` 1, `halfcantwidth` 1), dropping everything else. The result
has 32 blocks for Wilhelm2 and 53 for Zeppelin1, matching the fetched files and
the installed files block for block.

## Why these two

- `Hstr_6spur_Wilhelm2` is a one-way three-lane carriageway whose lanes are
  split over two asphalt textures (`str_asphdrk_1line`, `str_asphdrk_2line`),
  with a raised green strip on the left and a footway on the right. Its
  carriageway is not centred on the spline's origin: it runs from x = 0.000 to
  x = 8.900, so an import that drops the strips outside the carriageway still
  has to translate the result by 4.450 m.
- `str_4spur_zeppelin1` is a four-lane two-way road whose carriageway is split
  by a 3.500 m raised median (x = -1.750 to 1.750). Dropping the median shifts
  the inner lane paths by 1.125 m, so the median has to survive the import.
