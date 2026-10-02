//! RTTTL (Ring Tone Text Transfer Language) parser.
//!
//! RTTTL is a text format for encoding ringtones, originally used by Nokia phones.
//! This crate provides a no_std compatible parser for the format.
//!
//! # Format
//!
//! RTTTL format: `name:d=duration,o=octave,b=bpm:notes`
//!
//! - **name**: The melody name (before first colon)
//! - **d**: Default duration (1=whole, 2=half, 4=quarter, 8=eighth, 16, 32)
//! - **o**: Default octave (4-7 typically)
//! - **b**: BPM (beats per minute)
//! - **notes**: Comma-separated notes
//!
//! Each note: `[duration][note][#/b][octave][.]`
//!
//! # Example
//!
//! ```
//! use rtttl_rs::{Rtttl, Note, Duration};
//!
//! let rtttl = "test:d=4,o=5,b=120:c,d,e,f,g,a,b";
//! let parsed = Rtttl::parse(rtttl).unwrap();
//!
//! assert_eq!(parsed.name(), "test");
//! assert_eq!(parsed.bpm(), 120);
//! assert_eq!(parsed.notes().count(), 7);
//! ```
//!
//! # German Notation
//!
//! This parser supports German notation where 'H' represents B natural:
//! - H = B
//! - B = Bb (B flat)
//!
//! # no_std Support
//!
//! This crate is `no_std` by default. Enable the `std` feature for error trait implementations.

#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

/// Musical note pitch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Note {
    C = 0,
    Cs = 1, // C#
    D = 2,
    Ds = 3, // D#
    E = 4,
    F = 5,
    Fs = 6, // F#
    G = 7,
    Gs = 8, // G#
    A = 9,
    As = 10, // A#
    B = 11,
    Pause = 12,
}

impl Note {
    /// Get the note name.
    pub const fn name(self) -> &'static str {
        match self {
            Note::C => "C",
            Note::Cs => "C#",
            Note::D => "D",
            Note::Ds => "D#",
            Note::E => "E",
            Note::F => "F",
            Note::Fs => "F#",
            Note::G => "G",
            Note::Gs => "G#",
            Note::A => "A",
            Note::As => "A#",
            Note::B => "B",
            Note::Pause => "P",
        }
    }

    /// Get the chromatic index (0-11, or 12 for pause).
    pub const fn chromatic_index(self) -> u8 {
        self as u8
    }
}

/// Note duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Duration {
    Whole = 1,
    Half = 2,
    Quarter = 4,
    Eighth = 8,
    Sixteenth = 16,
    ThirtySecond = 32,
}

impl Duration {
    /// Create duration from RTTTL numeric value.
    pub const fn from_rtttl(value: u8) -> Option<Self> {
        match value {
            1 => Some(Duration::Whole),
            2 => Some(Duration::Half),
            4 => Some(Duration::Quarter),
            8 => Some(Duration::Eighth),
            16 => Some(Duration::Sixteenth),
            32 => Some(Duration::ThirtySecond),
            _ => None,
        }
    }

    /// Get the RTTTL numeric value.
    pub const fn to_rtttl(self) -> u8 {
        self as u8
    }

    /// Get the duration as a fraction of a whole note.
    pub const fn as_fraction(self) -> (u8, u8) {
        match self {
            Duration::Whole => (1, 1),
            Duration::Half => (1, 2),
            Duration::Quarter => (1, 4),
            Duration::Eighth => (1, 8),
            Duration::Sixteenth => (1, 16),
            Duration::ThirtySecond => (1, 32),
        }
    }
}

/// A parsed RTTTL note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtttlNote {
    /// The note pitch.
    pub note: Note,
    /// The octave (typically 4-7).
    pub octave: u8,
    /// The note duration.
    pub duration: Duration,
    /// Whether this note is dotted (1.5x duration).
    pub dotted: bool,
}

impl RtttlNote {
    /// Create a new note.
    pub const fn new(note: Note, octave: u8, duration: Duration) -> Self {
        Self {
            note,
            octave,
            duration,
            dotted: false,
        }
    }

    /// Create a dotted note.
    pub const fn dotted(note: Note, octave: u8, duration: Duration) -> Self {
        Self {
            note,
            octave,
            duration,
            dotted: true,
        }
    }

    /// Calculate duration in milliseconds given BPM.
    ///
    /// A whole note at 120 BPM = 2000ms (4 beats * 500ms per beat).
    pub const fn duration_ms(&self, bpm: u16) -> u32 {
        // Whole note duration = 4 beats = 4 * (60000 / bpm) ms
        let whole_note_ms = 240_000u32 / (bpm as u32);
        let base_ms = whole_note_ms / (self.duration as u32);

        if self.dotted {
            base_ms + (base_ms / 2)
        } else {
            base_ms
        }
    }
}

/// RTTTL parsing error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Missing colon separator in header.
    MissingColon,
    /// Invalid or missing settings section.
    InvalidSettings,
    /// Invalid note format.
    InvalidNote,
    /// Input is empty.
    Empty,
}

/// Parsed RTTTL melody header and note iterator.
#[derive(Debug, Clone)]
pub struct Rtttl<'a> {
    name: &'a str,
    default_duration: Duration,
    default_octave: u8,
    bpm: u16,
    notes_str: &'a str,
}

impl<'a> Rtttl<'a> {
    /// Parse an RTTTL string.
    pub fn parse(input: &'a str) -> Result<Self, ParseError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(ParseError::Empty);
        }

        // Find the two colons
        let first_colon = input.find(':').ok_or(ParseError::MissingColon)?;
        let name = &input[..first_colon];

        let rest = &input[first_colon + 1..];
        let second_colon = rest.find(':').ok_or(ParseError::MissingColon)?;

        let settings_str = &rest[..second_colon];
        let notes_str = &rest[second_colon + 1..];

        // Parse settings
        let mut default_duration = Duration::Quarter;
        let mut default_octave = 5u8;
        let mut bpm = 120u16;

        for setting in settings_str.split(',') {
            let setting = setting.trim();
            if let Some(val) = setting.strip_prefix("d=") {
                if let Some(d) = parse_number_u8(val).and_then(Duration::from_rtttl) {
                    default_duration = d;
                }
            } else if let Some(val) = setting.strip_prefix("o=") {
                if let Some(o) = parse_number_u8(val) {
                    default_octave = o;
                }
            } else if let Some(val) = setting.strip_prefix("b=") {
                if let Some(b) = parse_number_u16(val) {
                    bpm = b;
                }
            }
        }

        Ok(Self {
            name,
            default_duration,
            default_octave,
            bpm,
            notes_str,
        })
    }

    /// Get the melody name.
    pub const fn name(&self) -> &'a str {
        self.name
    }

    /// Get the default duration.
    pub const fn default_duration(&self) -> Duration {
        self.default_duration
    }

    /// Get the default octave.
    pub const fn default_octave(&self) -> u8 {
        self.default_octave
    }

    /// Get the BPM (beats per minute).
    pub const fn bpm(&self) -> u16 {
        self.bpm
    }

    /// Get an iterator over the notes.
    pub const fn notes(&self) -> NoteIterator<'a> {
        NoteIterator {
            remaining: self.notes_str,
            default_duration: self.default_duration,
            default_octave: self.default_octave,
        }
    }

    /// Count the total number of notes.
    pub fn note_count(&self) -> usize {
        self.notes().count()
    }

    /// Calculate total duration in milliseconds.
    pub fn total_duration_ms(&self) -> u32 {
        self.notes().map(|n| n.duration_ms(self.bpm)).sum()
    }
}

/// Iterator over RTTTL notes.
#[derive(Debug, Clone)]
pub struct NoteIterator<'a> {
    remaining: &'a str,
    default_duration: Duration,
    default_octave: u8,
}

impl Iterator for NoteIterator<'_> {
    type Item = RtttlNote;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Skip whitespace and find next note
            self.remaining = self.remaining.trim_start();
            if self.remaining.is_empty() {
                return None;
            }

            // Find the end of the current note (comma or end of string)
            let note_end = self.remaining.find(',').unwrap_or(self.remaining.len());
            let note_str = self.remaining[..note_end].trim();

            // Advance past this note
            self.remaining = if note_end < self.remaining.len() {
                &self.remaining[note_end + 1..]
            } else {
                ""
            };

            if note_str.is_empty() {
                continue;
            }

            if let Some(note) = parse_note(note_str, self.default_duration, self.default_octave) {
                return Some(note);
            }
            // Skip invalid notes and try next
        }
    }
}

/// Parse a single RTTTL note string.
fn parse_note(s: &str, default_duration: Duration, default_octave: u8) -> Option<RtttlNote> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let mut pos = 0;

    // Parse optional duration at start
    let mut duration_val = 0u8;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        duration_val = duration_val.saturating_mul(10).saturating_add(bytes[pos] - b'0');
        pos += 1;
    }

    let duration = if duration_val == 0 {
        default_duration
    } else {
        Duration::from_rtttl(duration_val).unwrap_or(default_duration)
    };

    if pos >= bytes.len() {
        return None;
    }

    // Parse note letter (case-insensitive)
    let note_char = bytes[pos].to_ascii_lowercase();
    pos += 1;

    // Check for sharp or flat
    let modifier = if pos < bytes.len() {
        match bytes[pos] {
            b'#' => {
                pos += 1;
                1i8
            }
            b'b' if note_char != b'b' && note_char != b'h' => {
                // 'b' as flat modifier (but not if the note itself is B or H)
                pos += 1;
                -1i8
            }
            _ => 0i8,
        }
    } else {
        0i8
    };

    // Parse the note
    // German notation: H = B natural, B = Bb
    let base_note = match note_char {
        b'c' => Note::C,
        b'd' => Note::D,
        b'e' => Note::E,
        b'f' => Note::F,
        b'g' => Note::G,
        b'a' => Note::A,
        b'b' => Note::As, // German: B = Bb
        b'h' => Note::B,  // German: H = B natural
        b'p' | b'r' => Note::Pause,
        _ => return None,
    };

    // Apply modifier (sharp/flat)
    let note = if base_note == Note::Pause {
        Note::Pause
    } else {
        let index = base_note.chromatic_index() as i8 + modifier;
        let index = ((index % 12) + 12) % 12; // Handle wraparound
        match index as u8 {
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
            _ => return None,
        }
    };

    // Parse optional octave
    let mut octave_val = 0u8;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        octave_val = octave_val.saturating_mul(10).saturating_add(bytes[pos] - b'0');
        pos += 1;
    }

    let octave = if octave_val == 0 {
        default_octave
    } else {
        octave_val
    };

    // Check for dotted note marker
    let dotted = pos < bytes.len() && bytes[pos] == b'.';

    Some(RtttlNote {
        note,
        octave,
        duration,
        dotted,
    })
}

/// Parse a number from the start of a string.
fn parse_number_u8(s: &str) -> Option<u8> {
    let mut result = 0u8;
    let mut found = false;
    for c in s.bytes() {
        if c.is_ascii_digit() {
            result = result.saturating_mul(10).saturating_add(c - b'0');
            found = true;
        } else {
            break;
        }
    }
    if found {
        Some(result)
    } else {
        None
    }
}

/// Parse a number from the start of a string.
fn parse_number_u16(s: &str) -> Option<u16> {
    let mut result = 0u16;
    let mut found = false;
    for c in s.bytes() {
        if c.is_ascii_digit() {
            result = result.saturating_mul(10).saturating_add((c - b'0') as u16);
            found = true;
        } else {
            break;
        }
    }
    if found {
        Some(result)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;

    #[test]
    fn parse_simple_melody() {
        let rtttl = "test:d=4,o=5,b=120:c,d,e,f,g,a,b";
        let parsed = Rtttl::parse(rtttl).unwrap();

        assert_eq!(parsed.name(), "test");
        assert_eq!(parsed.bpm(), 120);
        assert_eq!(parsed.default_octave(), 5);
        assert_eq!(parsed.default_duration(), Duration::Quarter);

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 7);
        assert_eq!(notes[0].note, Note::C);
        assert_eq!(notes[6].note, Note::As); // 'b' = Bb in German notation
    }

    #[test]
    fn parse_with_durations() {
        let rtttl = "test:d=4,o=5,b=120:1c,2d,4e,8f,16g,32a";
        let parsed = Rtttl::parse(rtttl).unwrap();

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 6);
        assert_eq!(notes[0].duration, Duration::Whole);
        assert_eq!(notes[1].duration, Duration::Half);
        assert_eq!(notes[2].duration, Duration::Quarter);
        assert_eq!(notes[3].duration, Duration::Eighth);
        assert_eq!(notes[4].duration, Duration::Sixteenth);
        assert_eq!(notes[5].duration, Duration::ThirtySecond);
    }

    #[test]
    fn parse_with_octaves() {
        let rtttl = "test:d=4,o=5,b=120:c4,c5,c6,c7";
        let parsed = Rtttl::parse(rtttl).unwrap();

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 4);
        assert_eq!(notes[0].octave, 4);
        assert_eq!(notes[1].octave, 5);
        assert_eq!(notes[2].octave, 6);
        assert_eq!(notes[3].octave, 7);
    }

    #[test]
    fn parse_sharps() {
        let rtttl = "test:d=4,o=5,b=120:c#,d#,f#,g#,a#";
        let parsed = Rtttl::parse(rtttl).unwrap();

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 5);
        assert_eq!(notes[0].note, Note::Cs);
        assert_eq!(notes[1].note, Note::Ds);
        assert_eq!(notes[2].note, Note::Fs);
        assert_eq!(notes[3].note, Note::Gs);
        assert_eq!(notes[4].note, Note::As);
    }

    #[test]
    fn parse_german_h_notation() {
        let rtttl = "test:d=4,o=5,b=120:h,h4,h#";
        let parsed = Rtttl::parse(rtttl).unwrap();

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].note, Note::B);
        assert_eq!(notes[1].note, Note::B);
        assert_eq!(notes[1].octave, 4);
        assert_eq!(notes[2].note, Note::C); // H# = C
    }

    #[test]
    fn parse_pauses() {
        let rtttl = "test:d=4,o=5,b=120:c,p,e,r,g";
        let parsed = Rtttl::parse(rtttl).unwrap();

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 5);
        assert_eq!(notes[1].note, Note::Pause);
        assert_eq!(notes[3].note, Note::Pause);
    }

    #[test]
    fn parse_dotted_notes() {
        let rtttl = "test:d=4,o=5,b=120:c.,d,e.";
        let parsed = Rtttl::parse(rtttl).unwrap();

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 3);
        assert!(notes[0].dotted);
        assert!(!notes[1].dotted);
        assert!(notes[2].dotted);
    }

    #[test]
    fn parse_doom() {
        let rtttl = "doom:b=420,o=4,d=32:8g4,8e4,8b3,8e4,4e,8p,1a#3";
        let parsed = Rtttl::parse(rtttl).unwrap();

        assert_eq!(parsed.name(), "doom");
        assert_eq!(parsed.bpm(), 420);
        assert_eq!(parsed.default_octave(), 4);

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 7);

        assert_eq!(notes[0].note, Note::G);
        assert_eq!(notes[0].octave, 4);
        assert_eq!(notes[0].duration, Duration::Eighth);

        assert_eq!(notes[2].note, Note::As); // b3 = Bb3
        assert_eq!(notes[2].octave, 3);

        assert_eq!(notes[5].note, Note::Pause);

        assert_eq!(notes[6].note, Note::As);
        assert_eq!(notes[6].octave, 3);
        assert_eq!(notes[6].duration, Duration::Whole);
    }

    #[test]
    fn parse_wam_melody() {
        let rtttl = "wam:b=116,o=5,d=1:16h4,16a4,4c";
        let parsed = Rtttl::parse(rtttl).unwrap();

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].note, Note::B); // H = B
        assert_eq!(notes[0].octave, 4);
        assert_eq!(notes[0].duration, Duration::Sixteenth);
    }

    #[test]
    fn duration_ms_calculation() {
        let note = RtttlNote::new(Note::C, 5, Duration::Quarter);
        // At 120 BPM: quarter note = 500ms
        assert_eq!(note.duration_ms(120), 500);

        let whole = RtttlNote::new(Note::C, 5, Duration::Whole);
        // At 120 BPM: whole note = 2000ms
        assert_eq!(whole.duration_ms(120), 2000);

        let dotted = RtttlNote::dotted(Note::C, 5, Duration::Quarter);
        // Dotted quarter at 120 BPM = 750ms
        assert_eq!(dotted.duration_ms(120), 750);
    }

    #[test]
    fn missing_colon_error() {
        assert_eq!(Rtttl::parse("invalid").unwrap_err(), ParseError::MissingColon);
        assert_eq!(Rtttl::parse("name:settings").unwrap_err(), ParseError::MissingColon);
    }

    #[test]
    fn empty_input_error() {
        assert_eq!(Rtttl::parse("").unwrap_err(), ParseError::Empty);
        assert_eq!(Rtttl::parse("   ").unwrap_err(), ParseError::Empty);
    }

    #[test]
    fn note_iterator_skips_invalid() {
        let rtttl = "test:d=4,o=5,b=120:c,,d,,e";
        let parsed = Rtttl::parse(rtttl).unwrap();

        let notes: Vec<_> = parsed.notes().collect();
        assert_eq!(notes.len(), 3);
    }
}
