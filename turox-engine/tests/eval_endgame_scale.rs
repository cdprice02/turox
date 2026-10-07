//! Concrete tests for `eval::endgame_scale`: which positions produce which
//! scale factor, and the `ScaleFactor` arithmetic on its own, so a change to
//! the fixed-point representation fails in an arithmetic test rather than as
//! unexplained drift in a position test.

use turox_chess::board::Board;
use turox_chess::strategies::mirrored;
use turox_engine::eval::endgame_scale::{scale_factor, ScaleFactor};
use turox_engine::eval::{eval_white_pov, Score};

// Every position below keeps the two kings on asymmetric squares (one
// centralized, one cornered) rather than mirroring each other: a
// self-mirror-symmetric *placement* here is also the bare-kings KK draw
// signature, which `endgame_scale::scale_factor` forces to exactly zero
// regardless of what the underlying score (including tempo) would have
// been on its own, so a mirrored pair of bare kings would pass even if
// `endgame_scale::scale` did nothing at all. Forcing a real, otherwise
// nonzero, king-PST or material asymmetry down to exactly 0 is what
// actually exercises the scale factor.
#[test]
fn bare_kings_score_exactly_zero_despite_asymmetric_king_placement() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/8 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 0);
}

#[test]
fn a_lone_knight_cannot_escape_the_draw_score() {
    let white_up_a_knight =
        Board::try_from_fen("7k/8/8/8/4K3/8/8/N7 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&white_up_a_knight), 0);

    // The asymmetric case this repo's {Color}x{side} history says to write
    // explicitly: the same signature with Black holding the extra knight,
    // not just White's own case mirrored.
    let black_up_a_knight =
        Board::try_from_fen("4k3/8/8/8/8/8/8/n6K w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&black_up_a_knight), 0);
}

#[test]
fn a_lone_bishop_cannot_escape_the_draw_score() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/B7 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 0);
}

#[test]
fn a_pair_of_knights_cannot_escape_the_draw_score() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/NN6 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 0);
}

// a1 and c1 are the same square color (both dark): `(0+0)` and `(2+0)` are
// both even under `Square::is_light`'s file+rank parity.
#[test]
fn same_colored_bishops_cannot_escape_the_draw_score() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/B1B5 w - - 0 1").expect("valid FEN");
    assert_eq!(eval_white_pov(&board), 0);
}

// The boundary same_colored_bishops_cannot_escape_the_draw_score sits next
// to: a1 and b1 are opposite square colors (`0` even, `1` odd), and a real
// king-and-two-opposite-coloured-bishops position is one of the four basic
// forced checkmates, not a draw. `hard_draw_scale` must leave this one
// alone rather than treating "two bishops, one side" as a single case.
#[test]
fn opposite_colored_bishops_on_one_side_are_not_scaled_to_a_draw() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/BB6 w - - 0 1").expect("valid FEN");
    assert!(eval_white_pov(&board) > 600); // two bishops' worth of material, roughly
}

// The other combination that can force mate despite being "only two
// minors": a knight and a bishop together, unlike either alone.
#[test]
fn a_knight_and_bishop_pair_are_not_scaled_to_a_draw() {
    let board = Board::try_from_fen("7k/8/8/8/4K3/8/8/BN6 w - - 0 1").expect("valid FEN");
    assert!(eval_white_pov(&board) > 600);
}

// White's bishop on b1 (file 1 + rank 0 = odd, light) and Black's on b8
// (file 1 + rank 7 = odd... file 1 + rank 7 = 8, even, dark): opposite
// colors, one each side, the classic fortress case. White's extra d4 pawn
// would be worth a full 100 centipawns plus its own positional terms
// unscaled; `soft_draw_scale` should leave White still (barely) ahead, but
// nowhere near a full pawn's worth.
#[test]
fn opposite_colored_bishops_with_an_extra_pawn_score_well_below_a_pawn() {
    let board = Board::try_from_fen("1b2k3/8/8/8/3P4/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    let score = eval_white_pov(&board);
    assert!(
        (0..50).contains(&score),
        "expected a small, positive, well-below-a-pawn score, got {score}"
    );

    // Same position, Black's own extra pawn instead of White's: the
    // {Color}x{side} check this repo's history says to write explicitly,
    // not just White's own case mirrored via `mirrored()`.
    let black_extra_pawn =
        Board::try_from_fen("1b2k3/8/8/3p4/8/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    let black_score = eval_white_pov(&black_extra_pawn);
    assert!(
        (-50..0).contains(&black_score),
        "expected a small, negative, well-below-a-pawn score, got {black_score}"
    );
}

// The far end of the ramp from the single-pawn case above: under the old
// flat 1/16 constant this four-pawn material lead would have scored around
// 25 (`4 * 100 / 16`), same as a one-pawn lead scaled down to about 6. The
// pawn-count-indexed curve leaves most of a lead this size intact instead,
// since CPW's framing treats a four-pawn opposite-bishop edge as often
// winning outright, not still a near-draw.
#[test]
fn a_four_pawn_ocb_advantage_scores_far_above_a_one_pawn_advantage() {
    let board = Board::try_from_fen("1b2k3/8/8/8/3PPPP1/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    let score = eval_white_pov(&board);
    assert!(
        score > 200,
        "expected the ramp to leave most of a four-pawn lead intact, got {score}"
    );

    // {Color}x{side}: Black holding the four-pawn edge instead of White,
    // not just White's own case mirrored via `mirrored()`.
    let black_board =
        Board::try_from_fen("1b2k3/3pppp1/8/8/8/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    let black_score = eval_white_pov(&black_board);
    assert!(
        black_score < -200,
        "expected the ramp to leave most of a four-pawn lead intact for Black too, got {black_score}"
    );

    assert_eq!(eval_white_pov(&mirrored(&board)), -score);
}

// The boundary the position above sits next to: same shape, but both
// bishops on the same square color (b1 and a8 are both light: `1+0=1` and
// `0+7=7`, both odd), so this isn't the opposite-coloured-bishops fortress
// at all. White's extra pawn should show through close to its full value,
// not scaled down.
#[test]
fn same_colored_bishops_with_an_extra_pawn_are_not_scaled_down() {
    let board = Board::try_from_fen("b3k3/8/8/8/3P4/8/8/1B2K3 w - - 0 1").expect("valid FEN");
    assert!(eval_white_pov(&board) > 80);
}

/// The identity has to be exact, including for negative scores: it is the
/// common case by far, so a rounding error here would be a constant small bias
/// on nearly every evaluation rather than a visible failure.
#[test]
fn an_unscaled_factor_leaves_every_score_untouched() {
    for score in [0, 1, -1, 7, -7, 999, -999, Score::MAX, Score::MIN] {
        assert_eq!(
            ScaleFactor::ONE.apply(score),
            score,
            "ScaleFactor::ONE must be the identity, and was not for {score}"
        );
    }
}

#[test]
fn a_draw_factor_zeroes_every_score() {
    for score in [0, 1, -1, 5000, -5000, Score::MAX, Score::MIN] {
        assert_eq!(ScaleFactor::DRAW.apply(score), 0);
    }
}

/// Truncation goes toward zero on both signs, matching the plain integer
/// division this replaced. Rounding away from zero would make a scaled-down
/// score drift *away* from the draw it is being pulled toward.
#[test]
fn scaling_truncates_toward_zero_on_both_signs() {
    let sixteenth = ScaleFactor::from_reciprocal(16);

    assert_eq!(sixteenth.apply(160), 10);
    assert_eq!(sixteenth.apply(-160), -10);
    // 100/16 is 6.25: both signs land on 6, not 7 and not -7.
    assert_eq!(sixteenth.apply(100), 6);
    assert_eq!(sixteenth.apply(-100), -6);
}

/// A zero divisor is meaningless rather than catastrophic, and saturates to a
/// draw instead of dividing by zero. Worth pinning because the queued scale
/// rules build their divisors from position features, so a zero is reachable
/// by arithmetic rather than only by someone typing it.
#[test]
fn a_zero_divisor_is_a_draw_rather_than_a_panic() {
    assert_eq!(ScaleFactor::from_reciprocal(0), ScaleFactor::DRAW);
    assert_eq!(ScaleFactor::from_reciprocal(0).apply(500), 0);
}

/// A larger divisor scales further down, monotonically. This is the property
/// every future rule depends on, since each one picks a divisor from position
/// features and expects "more unwinnable" to mean "smaller score".
#[test]
fn a_larger_divisor_never_scales_less() {
    let score = 3200;
    let mut previous = ScaleFactor::ONE.apply(score);
    for divisor in [1, 2, 4, 8, 16, 32, 64] {
        let scaled = ScaleFactor::from_reciprocal(divisor).apply(score);
        assert!(
            scaled <= previous,
            "divisor {divisor} scaled {score} to {scaled}, above the previous {previous}"
        );
        previous = scaled;
    }
}

/// `from_numerator` is the general-curve counterpart to `from_reciprocal`:
/// below `UNIT` it produces exactly the fraction asked for, matching a
/// `from_reciprocal` call with the same effective ratio.
#[test]
fn from_numerator_below_unit_matches_the_equivalent_reciprocal() {
    assert_eq!(
        ScaleFactor::from_numerator(16),
        ScaleFactor::from_reciprocal(16)
    );
    assert_eq!(ScaleFactor::from_numerator(128).apply(256), 128);
}

/// A numerator at or past `UNIT` clamps to `ScaleFactor::ONE` rather than
/// amplifying the score: nothing in this module ever scales a score up, so a
/// curve that overshoots means "unscaled", not "boosted".
#[test]
fn from_numerator_at_or_above_unit_clamps_to_one() {
    assert_eq!(ScaleFactor::from_numerator(256), ScaleFactor::ONE);
    assert_eq!(ScaleFactor::from_numerator(1000), ScaleFactor::ONE);
}

/// The pawn-count-indexed opposite-coloured-bishops ramp, checked at the
/// `scale_factor` level rather than through a private helper: a pawn
/// difference of 0 or 1 reproduces the original flat 1/16, a difference of
/// 5 or more reaches fully unscaled, and every step in between is at least
/// as generous as the last. This is the property every future curve tweak
/// depends on, since a smaller pawn edge should never end up scaled *up*
/// relative to a larger one.
#[test]
fn a_larger_ocb_pawn_advantage_never_scales_less() {
    let pawn_ranks = ["8", "3P4", "3PP3", "3PPP2", "3PPPP1", "3PPPPP", "2PPPPPP"];
    let boards: Vec<Board> = pawn_ranks
        .iter()
        .map(|pawns| {
            let fen = format!("1b2k3/8/8/{pawns}/8/8/8/1B2K3 w - - 0 1");
            Board::try_from_fen(&fen).expect("valid FEN")
        })
        .collect();

    let sixteenth = ScaleFactor::from_reciprocal(16);
    assert_eq!(
        scale_factor(&boards[0]),
        sixteenth,
        "a 0-pawn edge should match the old flat constant"
    );
    assert_eq!(
        scale_factor(&boards[1]),
        sixteenth,
        "a 1-pawn edge should match the old flat constant"
    );
    assert_eq!(
        scale_factor(&boards[5]),
        ScaleFactor::ONE,
        "a 5-pawn edge should be fully unscaled"
    );
    assert_eq!(
        scale_factor(&boards[6]),
        ScaleFactor::ONE,
        "a 6-pawn edge should stay fully unscaled"
    );

    let mut previous = ScaleFactor::DRAW;
    for (pawns, board) in pawn_ranks.iter().zip(&boards) {
        let factor = scale_factor(board);
        assert!(
            factor >= previous,
            "pawn rank {pawns:?} scaled to {factor:?}, below the previous {previous:?}"
        );
        previous = factor;
    }
}
