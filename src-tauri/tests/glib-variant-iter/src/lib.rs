//! Behavior checks for the GLib string iterator backport.
//!
//! Run with `cargo test --release -p syncmc-glib-regression`: optimizations can
//! expose the immutable out-pointer bug hidden by unoptimized builds.

#[cfg(test)]
mod tests {
    use glib::{prelude::*, Variant};

    fn strings(values: &[&str]) -> Variant {
        Variant::array_from_iter::<String>(values.iter().map(|value| value.to_variant()))
    }

    #[test]
    fn reads_empty_and_unicode_strings_in_both_directions() {
        let values = ["", "Minecraft", "Schnee ❄", "世界", "🦀"];
        let variant = strings(&values);

        assert_eq!(
            variant.array_iter_str().unwrap().collect::<Vec<_>>(),
            values
        );
        assert_eq!(
            variant.array_iter_str().unwrap().rev().collect::<Vec<_>>(),
            values.into_iter().rev().collect::<Vec<_>>()
        );
    }

    #[test]
    fn interleaves_front_and_back_without_losing_strings() {
        let variant = strings(&["", "Mitte", "雪", "🦀"]);
        let mut iter = variant.array_iter_str().unwrap();

        assert_eq!(iter.next(), Some(""));
        assert_eq!(iter.next_back(), Some("🦀"));
        assert_eq!(iter.next_back(), Some("雪"));
        assert_eq!(iter.next(), Some("Mitte"));
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next_back(), None);
    }

    #[test]
    fn skips_from_both_ends_and_reads_the_remaining_strings() {
        let variant = strings(&["zero", "one", "two", "three", "four", "five"]);
        let mut iter = variant.array_iter_str().unwrap();

        assert_eq!(iter.nth(1), Some("one"));
        assert_eq!(iter.nth_back(1), Some("four"));
        assert_eq!(iter.next(), Some("two"));
        assert_eq!(iter.next_back(), Some("three"));
        assert_eq!(iter.nth(0), None);
        assert_eq!(iter.nth_back(0), None);
    }

    #[test]
    fn last_reads_the_final_remaining_string() {
        let variant = strings(&["first", "世界", "last"]);
        assert_eq!(variant.array_iter_str().unwrap().last(), Some("last"));

        let mut iter = variant.array_iter_str().unwrap();
        assert_eq!(iter.next(), Some("first"));
        assert_eq!(iter.next_back(), Some("last"));
        assert_eq!(iter.last(), Some("世界"));
    }

    #[test]
    fn empty_and_exhausted_iterators_remain_empty() {
        let empty = strings(&[]);
        let mut iter = empty.array_iter_str().unwrap();
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next_back(), None);
        assert_eq!(iter.nth(0), None);
        assert_eq!(iter.nth_back(0), None);
        assert_eq!(iter.last(), None);

        let singleton = strings(&[""]);
        let mut iter = singleton.array_iter_str().unwrap();
        assert_eq!(iter.next_back(), Some(""));
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next_back(), None);
        assert_eq!(iter.last(), None);
    }
}
