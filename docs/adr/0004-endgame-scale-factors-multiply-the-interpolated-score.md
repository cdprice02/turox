# Endgame scale factors multiply the interpolated score, not a tapered-sum term

Material and positional terms (material+PST, pawn structure, king safety) are
each summed into the `Tapered` mg/eg accumulator and blended once by
`phase::interpolate`. Endgame scale factors (starting with known draws and
opposite-coloured bishops) instead multiply that already-interpolated `Score`
afterward, in a separate `eval::endgame_scale` step.

## Considered options

Folding a scale factor into the tapered sum as another term can't express
"this material balance is a draw regardless of position": an additive term
still leaves every other term's contribution intact, so the total would only
reach zero if that one term happened to exactly cancel the rest, at every
phase. A post-interpolation multiplier does it directly: multiplying by zero
mutes whatever the summed terms produced, and multiplying by a fraction dulls
it without needing to know what those terms were. That matches what this
mechanism is for: overriding the rest of eval for a known-unwinnable material
balance, not competing with it as one term among several.
