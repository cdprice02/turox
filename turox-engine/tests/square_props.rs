//! Property tests for `Square`, `File`, and `Rank`. Unlike the `Bitboard` exercise,
//! these types are fully implemented already, so none of these are `#[ignore]`d.

use proptest::prelude::*;
use turox_engine::Square;

fn any_square() -> impl Strategy<Value = Square> {
    (0u8..64).prop_map(|i| Square::from_index(i).expect("i in 0..64"))
}

proptest! {
    #[test]
    fn from_index_round_trips_with_index(sq in any_square()) {
        prop_assert_eq!(Square::from_index(sq.index()), Some(sq));
    }

    #[test]
    fn new_round_trips_with_file_and_rank(sq in any_square()) {
        prop_assert_eq!(Square::new(sq.file(), sq.rank()), sq);
    }

    #[test]
    fn flip_rank_is_an_involution(sq in any_square()) {
        prop_assert_eq!(sq.flip_rank().flip_rank(), sq);
    }

    #[test]
    fn flip_file_is_an_involution(sq in any_square()) {
        prop_assert_eq!(sq.flip_file().flip_file(), sq);
    }

    #[test]
    fn flip_rank_preserves_file(sq in any_square()) {
        prop_assert_eq!(sq.flip_rank().file(), sq.file());
    }

    #[test]
    fn flip_file_preserves_rank(sq in any_square()) {
        prop_assert_eq!(sq.flip_file().rank(), sq.rank());
    }

    #[test]
    fn distance_is_symmetric(a in any_square(), b in any_square()) {
        prop_assert_eq!(a.distance(b), b.distance(a));
    }

    #[test]
    fn distance_is_zero_only_for_equal_squares(a in any_square(), b in any_square()) {
        prop_assert_eq!(a.distance(b) == 0, a == b);
    }

    #[test]
    fn offset_zero_zero_is_identity(sq in any_square()) {
        prop_assert_eq!(sq.offset(0, 0), Some(sq));
    }

    #[test]
    fn offset_out_of_range_is_none(sq in any_square(), df in -20i8..=20, dr in -20i8..=20) {
        let file = sq.file().index() as i8 + df;
        let rank = sq.rank().index() as i8 + dr;
        if !(0..=7).contains(&file) || !(0..=7).contains(&rank) {
            prop_assert_eq!(sq.offset(df, dr), None);
        }
    }

    #[test]
    fn algebraic_display_round_trips_through_file_rank(sq in any_square()) {
        let s = sq.to_string();
        let mut chars = s.chars();
        let file_ch = chars.next().unwrap();
        let rank_ch = chars.next().unwrap();
        prop_assert_eq!(file_ch, (b'a' + sq.file().index()) as char);
        prop_assert_eq!(rank_ch.to_digit(10).unwrap() as u8, sq.rank().index() + 1);
    }
}
