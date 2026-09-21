# PVS excludes quiescence, and trusts its own PV table over the TT

Principal variation search's null-window probe, its conditional re-search,
and the `is_pv` propagation that drives them apply only to `negamax` and
`search_root`. Quiescence's two move loops keep the uniform, single-window
treatment they always had, with no notion of `is_pv` at all. And the move
this engine trusts for a node's principal-variation slot in `MovePriority`
comes from `Search::pv`, an in-tree table built and cleared alongside the
search itself, not from repeatedly probing the transposition table for the
position at each ply.

## Considered options

**Extend PVS's windowing into quiescence too.** The horizon is already a
leaf-like search bounded by `MAX_QUIESCENCE_DEPTH`, and a null-window probe
there would rarely change which capture gets tried, since ordering at that
depth already leans on MVV-LVA and the shared killer/history tables. Rejected
for the return on the added surface: a fourth call site threading `is_pv`
through `alpha_beta_loop`'s shared windowing logic, for a horizon where the
technique's own justification (avoiding a costly full-window search of a
move that probably isn't best) barely applies, since quiescence's move lists
are already short and already well-ordered.

**Reconstruct the PV by probing the TT at each ply after search completes,
instead of maintaining a separate table.** No new state on `Search`, and the
TT already stores a best move per position. Rejected because the TT's
replacement policy can overwrite an entry between when a line was searched
and when the PV would be read back, so a probed-out line can repeat a
position, skip one, or simply diverge from what the search actually found.
`Search::pv` is built and cleared during the search itself, so what it
reports is exactly what produced the score being reported next to it, never
a reconstruction that can drift from it.

## Consequences

`MovePriority::PrincipalVariation` and `Hash` can each name a different move
for the same position: the hash move is whatever the TT currently holds,
subject to replacement, while the PV move is this search's own record. The
PV line reported for a mate found through a null-window cutoff (not a
re-search) still needs that cutoff re-searched with the real window purely
to populate `Search::pv`, at whichever PV node it occurred; the SPRT gate
this landed under measured that cost as net positive already included, not
as a separate line item to watch later. Quiescence's own move choices never
appear in `info pv`; a reported line ends at the main search horizon exactly
where quiescence takes over, one ply short of whatever capture sequence
quiescence actually resolved.
