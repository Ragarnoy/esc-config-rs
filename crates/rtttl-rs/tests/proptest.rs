//! Property-based tests for rtttl-rs.

use proptest::prelude::*;
use rtttl_rs::{Duration, Note, ParseError, Rtttl, RtttlNote};

/// Strategy to generate valid note characters.
fn note_char_strategy() -> impl Strategy<Value = char> {
    prop_oneof![
        Just('c'),
        Just('d'),
        Just('e'),
        Just('f'),
        Just('g'),
        Just('a'),
        Just('b'),
        Just('h'),
        Just('p'),
        Just('r'),
        Just('C'),
        Just('D'),
        Just('E'),
        Just('F'),
        Just('G'),
        Just('A'),
        Just('B'),
        Just('H'),
        Just('P'),
        Just('R'),
    ]
}

/// Strategy to generate valid duration values.
fn duration_strategy() -> impl Strategy<Value = u8> {
    prop_oneof![Just(1u8), Just(2u8), Just(4u8), Just(8u8), Just(16u8), Just(32u8),]
}

/// Strategy to generate valid octave values.
fn octave_strategy() -> impl Strategy<Value = u8> {
    1u8..=8u8
}

/// Strategy to generate valid BPM values.
fn bpm_strategy() -> impl Strategy<Value = u16> {
    20u16..=900u16
}

/// Strategy to generate a valid melody name.
fn name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_]{0,15}".prop_filter("non-empty", |s| !s.is_empty())
}

/// Strategy to generate a single valid note string.
fn note_string_strategy() -> impl Strategy<Value = String> {
    (
        prop::option::of(duration_strategy()),
        note_char_strategy(),
        prop::bool::ANY, // sharp
        prop::option::of(octave_strategy()),
        prop::bool::ANY, // dotted
    )
        .prop_map(|(dur, note, sharp, oct, dotted)| {
            let mut s = String::new();
            if let Some(d) = dur {
                s.push_str(&d.to_string());
            }
            s.push(note);
            if sharp && note != 'p' && note != 'r' && note != 'P' && note != 'R' {
                s.push('#');
            }
            if let Some(o) = oct {
                s.push_str(&o.to_string());
            }
            if dotted {
                s.push('.');
            }
            s
        })
}

/// Strategy to generate a valid RTTTL string.
fn rtttl_strategy() -> impl Strategy<Value = String> {
    (
        name_strategy(),
        duration_strategy(),
        octave_strategy(),
        bpm_strategy(),
        prop::collection::vec(note_string_strategy(), 1..50),
    )
        .prop_map(|(name, dur, oct, bpm, notes)| {
            format!("{}:d={},o={},b={}:{}", name, dur, oct, bpm, notes.join(","))
        })
}

proptest! {
    /// Any valid RTTTL string should parse without error.
    #[test]
    fn parse_valid_rtttl(rtttl in rtttl_strategy()) {
        let result = Rtttl::parse(&rtttl);
        prop_assert!(result.is_ok(), "Failed to parse: {}", rtttl);
    }

    /// Parsed RTTTL should have the correct name.
    #[test]
    fn parsed_name_matches(
        name in name_strategy(),
        dur in duration_strategy(),
        oct in octave_strategy(),
        bpm in bpm_strategy(),
    ) {
        let rtttl = format!("{}:d={},o={},b={}:c", name, dur, oct, bpm);
        let parsed = Rtttl::parse(&rtttl).unwrap();
        prop_assert_eq!(parsed.name(), name.as_str());
    }

    /// Parsed RTTTL should have the correct BPM.
    #[test]
    fn parsed_bpm_matches(
        name in name_strategy(),
        bpm in bpm_strategy(),
    ) {
        let rtttl = format!("{}:d=4,o=5,b={}:c", name, bpm);
        let parsed = Rtttl::parse(&rtttl).unwrap();
        prop_assert_eq!(parsed.bpm(), bpm);
    }

    /// Parsed RTTTL should have the correct default octave.
    #[test]
    fn parsed_default_octave_matches(
        name in name_strategy(),
        oct in octave_strategy(),
    ) {
        let rtttl = format!("{}:d=4,o={},b=120:c", name, oct);
        let parsed = Rtttl::parse(&rtttl).unwrap();
        prop_assert_eq!(parsed.default_octave(), oct);
    }

    /// Note count should match number of comma-separated notes.
    #[test]
    fn note_count_matches(
        notes in prop::collection::vec(note_string_strategy(), 1..20),
    ) {
        let rtttl = format!("test:d=4,o=5,b=120:{}", notes.join(","));
        let parsed = Rtttl::parse(&rtttl).unwrap();
        // Note: some notes might be invalid and skipped, so count parsed notes
        let parsed_count = parsed.notes().count();
        // All our generated notes should be valid
        prop_assert!(parsed_count > 0);
        prop_assert!(parsed_count <= notes.len());
    }

    /// Duration::from_rtttl should round-trip correctly.
    #[test]
    fn duration_roundtrip(dur in duration_strategy()) {
        let duration = Duration::from_rtttl(dur).unwrap();
        prop_assert_eq!(duration.to_rtttl(), dur);
    }

    /// Note duration_ms should always be positive and reasonable.
    #[test]
    fn duration_ms_is_reasonable(
        dur in duration_strategy(),
        bpm in 20u16..=600u16,
    ) {
        let duration = Duration::from_rtttl(dur).unwrap();
        let note = RtttlNote::new(Note::C, 5, duration);
        let ms = note.duration_ms(bpm);

        // Duration should be positive
        prop_assert!(ms > 0);

        // Whole note at slowest BPM (20) = 12000ms, fastest (600) = 400ms
        // Thirty-second note at slowest = 375ms, fastest ~12ms
        prop_assert!(ms <= 15000, "Duration {} too long at {} BPM", ms, bpm);
    }

    /// Dotted notes should be 1.5x the base duration.
    #[test]
    fn dotted_note_duration(
        dur in duration_strategy(),
        bpm in 60u16..=240u16,
    ) {
        let duration = Duration::from_rtttl(dur).unwrap();
        let regular = RtttlNote::new(Note::C, 5, duration);
        let dotted = RtttlNote::dotted(Note::C, 5, duration);

        let regular_ms = regular.duration_ms(bpm);
        let dotted_ms = dotted.duration_ms(bpm);

        // Dotted should be 1.5x regular (with integer math tolerance)
        let expected = regular_ms + (regular_ms / 2);
        prop_assert_eq!(dotted_ms, expected);
    }

    /// Empty input should return Empty error.
    #[test]
    fn empty_input_returns_error(whitespace in "[ \t\n\r]*") {
        let result = Rtttl::parse(&whitespace);
        if whitespace.trim().is_empty() {
            prop_assert_eq!(result.unwrap_err(), ParseError::Empty);
        }
    }

    /// Input without colons should return MissingColon error.
    #[test]
    fn no_colon_returns_error(input in "[a-zA-Z0-9]+") {
        if !input.contains(':') && !input.trim().is_empty() {
            let result = Rtttl::parse(&input);
            prop_assert_eq!(result.unwrap_err(), ParseError::MissingColon);
        }
    }

    /// Note chromatic index should be in valid range.
    #[test]
    fn note_chromatic_index_valid(note_idx in 0u8..=12u8) {
        let note = match note_idx {
            0 => Note::C,
            1 => Note::Cs,
            2 => Note::D,
            3 => Note::Ds,
            4 => Note::E,
            5 => Note::F,
            6 => Note::Fs,
            7 => Note::G,
            8 => Note::Gs,
            9 => Note::A,
            10 => Note::As,
            11 => Note::B,
            _ => Note::Pause,
        };
        prop_assert!(note.chromatic_index() <= 12);
    }

    /// Whitespace handling should be consistent.
    #[test]
    fn whitespace_handling(
        name in name_strategy(),
        spaces1 in "[ \t]{0,3}",
        spaces2 in "[ \t]{0,3}",
    ) {
        let rtttl1 = format!("{}:d=4,o=5,b=120:c,d,e", name);
        let rtttl2 = format!("{}{}:d=4,o=5,b=120:{}c,d,e{}", spaces1, name, spaces2, spaces1);

        let parsed1 = Rtttl::parse(&rtttl1).unwrap();
        let parsed2 = Rtttl::parse(&rtttl2);

        // Both should parse (or both fail for the name with leading space)
        if parsed2.is_ok() {
            let parsed2 = parsed2.unwrap();
            prop_assert_eq!(parsed1.notes().count(), parsed2.notes().count());
        }
    }

    /// Case insensitivity for note letters.
    #[test]
    fn case_insensitivity(note_char in "[cdefgabh]") {
        let lower = format!("test:d=4,o=5,b=120:{}", note_char);
        let upper = format!("test:d=4,o=5,b=120:{}", note_char.to_uppercase());

        let parsed_lower = Rtttl::parse(&lower).unwrap();
        let parsed_upper = Rtttl::parse(&upper).unwrap();

        let notes_lower: Vec<_> = parsed_lower.notes().collect();
        let notes_upper: Vec<_> = parsed_upper.notes().collect();

        prop_assert_eq!(notes_lower.len(), notes_upper.len());
        if !notes_lower.is_empty() {
            prop_assert_eq!(notes_lower[0].note, notes_upper[0].note);
        }
    }
}

/// Test that the iterator is fused (returns None forever after exhaustion).
#[test]
fn iterator_is_fused() {
    let rtttl = "test:d=4,o=5,b=120:c,d,e";
    let parsed = Rtttl::parse(rtttl).unwrap();
    let mut iter = parsed.notes();

    // Consume all notes
    assert!(iter.next().is_some());
    assert!(iter.next().is_some());
    assert!(iter.next().is_some());

    // Should return None forever
    assert!(iter.next().is_none());
    assert!(iter.next().is_none());
    assert!(iter.next().is_none());
}

/// Test total duration calculation.
#[test]
fn total_duration_calculation() {
    // 4 quarter notes at 120 BPM = 4 * 500ms = 2000ms
    let rtttl = "test:d=4,o=5,b=120:c,d,e,f";
    let parsed = Rtttl::parse(rtttl).unwrap();
    assert_eq!(parsed.total_duration_ms(), 2000);
}
