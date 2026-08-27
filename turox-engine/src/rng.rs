//! A deterministic, fixed-seed PRNG. Crate-private: this exists to make search
//! results (magic numbers, and eventually Zobrist keys) reproducible across runs
//! and platforms, not for anything an outside caller should reach for.
//!
//! `xorshift64star` is the algorithm, not an implementation choice up for grabs:
//! Sebastiano Vigna's xorshift64* (2014), the same generator Stockfish uses for
//! exactly this job (magic-number search). The exact shift amounts (12, 25, 27)
//! and multiplier below are part of that specification, not tuning knobs; getting
//! any of them wrong doesn't produce a compile error or an obviously-broken
//! result, just a worse-quality (or, at the multiplier, differently-seeded)
//! sequence, so `tests::matches_known_answers` below pins the exact numeric
//! output against known-answer test vectors, cross-checked outside this crate,
//! rather than trusting it by inspection.
//!
//! `board/zobrist.rs`'s TODO already commits to needing a fixed-seed const PRNG
//! for its key table; this is that PRNG's home. `benches/bitboard.rs`'s local
//! `XorShift64` (used only to generate varied bench inputs) can't fold into this
//! the same way, despite the similar shape: benches compile as their own binary,
//! like `tests/*.rs`, so they only see `pub` API. `xorshift64star` staying
//! crate-private is deliberate (see above), which means it stays invisible there
//! regardless. That duplication is fine; only the *reproducible-across-runs*
//! use case (magics, Zobrist keys) needs the real thing.
//!
//! # Gotcha: zero is a fixed point
//!
//! Like any xorshift variant, `0` maps to `0` forever: every step is `x ^= x >>
//! /<< n`, and XOR-ing zero with anything computed from zero is still zero. A
//! seed (or any intermediate state) of `0` silently produces an all-zero
//! sequence instead of failing loudly, so callers must guarantee a nonzero seed;
//! this function has no way to detect or recover from a zero input itself.

/// One step of xorshift64*, advancing `state` to the next value in the sequence.
/// Deterministic: the same `state` always produces the same output, on any
/// platform, in any Rust version. That reproducibility is the entire point of
/// hand-rolling this instead of using a real RNG crate. See the module doc for
/// the zero-is-a-fixed-point gotcha; `state` must be nonzero. Wired up by
/// `move_gen::magic::regen`'s magic search, the only caller; that module is
/// `#[cfg(test)]`-gated, so a plain (non-test) build has no caller at all.
#[allow(dead_code)] // only called from regen, which is #[cfg(test)]-only
pub(crate) const fn xorshift64star(state: u64) -> u64 {
    let mut x = state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-answer test: `(seed, expected)` pairs for xorshift64*, cross-checked
    /// against two independent implementations run outside this crate (a Python
    /// script and a standalone `rustc`-compiled binary, neither importing this
    /// file) before being hardcoded here. A same-formula Rust function living in
    /// this same test module was tried first and rejected: it was a
    /// line-for-line duplicate of `xorshift64star`'s body, so a transcription
    /// error made once (wrong shift amount, wrong multiplier) would very
    /// plausibly get made identically in both copies, defeating the point.
    /// Fixed-answer pairs, not a formula, are the only real independent check
    /// for something fully specified like this.
    const KNOWN_ANSWERS: [(u64, u64); 5] = [
        (0x0000_0000_0000_0001, 0x47e4_ce4b_896c_dd1d),
        (0x0000_0000_0000_002a, 0x56ce_4ab7_719b_a3a0),
        (0x0000_0000_dead_beef, 0x4615_1251_b681_bada),
        (0xffff_ffff_ffff_ffff, 0xf92c_c9e5_c600_0000),
        (0x9e37_79b9_7f4a_7c15, 0x0d83_b3e2_9a21_487a),
    ];

    #[test]
    fn matches_known_answers() {
        // Not proptest-generated: proptest's `impl Iterator` machinery and
        // shrinking aren't `const fn` friendly, and a known-answer test is
        // fixed data by definition, not something to generate.
        for (seed, expected) in KNOWN_ANSWERS {
            assert_eq!(xorshift64star(seed), expected, "seed {seed:#x}");
        }
    }

    #[test]
    fn never_produces_zero_from_a_nonzero_seed() {
        // Not a general xorshift theorem (some variants do hit 0 from certain
        // seeds), but true for xorshift64* specifically, since it's a bijection
        // on the nonzero 64-bit values (a maximal-period generator over 2^64 -
        // 1 states) composed with a multiply by an odd constant, which is also
        // a bijection on all 64-bit values and so never maps a nonzero input to
        // 0. Walk the sequence a while from a few different seeds to spot-check
        // it, rather than proving the bijection property here.
        for seed in [1u64, 42, 0xDEAD_BEEF, u64::MAX] {
            let mut state = seed;
            for _ in 0..1000 {
                state = xorshift64star(state);
                assert_ne!(state, 0, "seed {seed:#x} produced 0 after some steps");
            }
        }
    }

    #[test]
    fn zero_seed_stays_zero() {
        // Documents the gotcha in the module doc as an executable fact, not a
        // property to fix: this is the trap callers must avoid by construction
        // (nonzero seed), not something this function can detect.
        assert_eq!(xorshift64star(0), 0);
    }
}
