#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Size {
    pub fn area(self) -> usize {
        usize::from(self.rows) * usize::from(self.cols)
    }
}

#[cfg(test)]
mod tests {

    use crate::geom::Dir;
    use crate::geom::SplitDir;

    #[test]
    fn a_split_direction_round_trips_through_its_spelling() {
        for dir in [SplitDir::Horizontal, SplitDir::Vertical] {
            assert_eq!(SplitDir::parse(&dir.to_string()), Some(dir));
        }
    }

    #[test]
    fn a_direction_round_trips_through_its_spelling_and_knows_its_orientation() {
        for dir in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
            assert_eq!(Dir::parse(&dir.to_string()), Some(dir));
        }
        assert_eq!(Dir::Left.orientation(), SplitDir::Vertical);
        assert_eq!(Dir::Down.orientation(), SplitDir::Horizontal);
        assert!(Dir::Right.from_first_child());
        assert!(!Dir::Up.from_first_child());
        assert_eq!(Dir::parse("sideways"), None);
    }

    #[test]
    fn an_unknown_spelling_is_not_a_direction() {
        assert_eq!(SplitDir::parse("sideways"), None);
        assert_eq!(SplitDir::parse("Horizontal"), None);
    }
}
