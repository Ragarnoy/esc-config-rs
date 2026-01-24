//! Melody/music support for AM32 ESCs.
//!
//! AM32 ESCs can play custom startup melodies stored in EEPROM.
//! Each note is encoded as a single byte with pitch, octave, and duration.
//!
//! # RTTTL Support
//!
//! This module can parse RTTTL (Ring Tone Text Transfer Language) format
//! via the `rtttl-rs` crate. Example:
//!
//! ```text
//! doom:b=420,o=4,d=32:8g4,8e4,8b3,4e,8p,1a#3
//! ```
//!
//! Note: AM32 only supports octaves 4-7 and durations whole/half/quarter/eighth.
//! Notes outside this range are transposed, and 16th/32nd notes become eighths.

use core::fmt;

/// Offset from config start to melody data.
pub const MELODY_OFFSET: usize = 48;

/// Size of melody data in bytes.
pub const MELODY_SIZE: usize = 128;

/// Maximum number of notes in a melody.
pub const MELODY_MAX_NOTES: usize = 127; // Last byte reserved for terminator

/// Value indicating no melody is configured.
pub const MELODY_EMPTY: u8 = 0xFF;

/// Musical note (chromatic scale).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    C,
    Cs, // C#
    D,
    Ds, // D#
    E,
    F,
    Fs, // F#
    G,
    Gs, // G#
    A,
    As, // A#
    B,
    Pause,
}

impl Note {
    /// Create note from the upper nibble of a melody byte.
    pub fn from_nibble(n: u8) -> Self {
        match n & 0x0F {
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
        }
    }

    /// Convert note to nibble value.
    pub fn to_nibble(self) -> u8 {
        match self {
            Note::C => 0,
            Note::Cs => 1,
            Note::D => 2,
            Note::Ds => 3,
            Note::E => 4,
            Note::F => 5,
            Note::Fs => 6,
            Note::G => 7,
            Note::Gs => 8,
            Note::A => 9,
            Note::As => 10,
            Note::B => 11,
            Note::Pause => 12,
        }
    }

    /// Get display name for the note.
    pub const fn name(&self) -> &'static str {
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

    /// Convert from rtttl-rs Note type.
    fn from_rtttl(note: rtttl_rs::Note) -> Self {
        match note {
            rtttl_rs::Note::C => Note::C,
            rtttl_rs::Note::Cs => Note::Cs,
            rtttl_rs::Note::D => Note::D,
            rtttl_rs::Note::Ds => Note::Ds,
            rtttl_rs::Note::E => Note::E,
            rtttl_rs::Note::F => Note::F,
            rtttl_rs::Note::Fs => Note::Fs,
            rtttl_rs::Note::G => Note::G,
            rtttl_rs::Note::Gs => Note::Gs,
            rtttl_rs::Note::A => Note::A,
            rtttl_rs::Note::As => Note::As,
            rtttl_rs::Note::B => Note::B,
            rtttl_rs::Note::Pause => Note::Pause,
        }
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Note duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Duration {
    Whole = 0,
    Half = 1,
    Quarter = 2,
    Eighth = 3,
}

impl Duration {
    /// Create duration from 2-bit value.
    pub fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Duration::Whole,
            1 => Duration::Half,
            2 => Duration::Quarter,
            _ => Duration::Eighth,
        }
    }

    /// Get display name for the duration.
    pub const fn name(&self) -> &'static str {
        match self {
            Duration::Whole => "whole",
            Duration::Half => "half",
            Duration::Quarter => "quarter",
            Duration::Eighth => "eighth",
        }
    }

    /// Get short symbol for the duration.
    pub const fn symbol(&self) -> &'static str {
        match self {
            Duration::Whole => "1",
            Duration::Half => "2",
            Duration::Quarter => "4",
            Duration::Eighth => "8",
        }
    }

    /// Convert from rtttl-rs Duration type.
    ///
    /// AM32 only supports whole through eighth, so 16th and 32nd become eighth.
    fn from_rtttl(dur: rtttl_rs::Duration) -> Self {
        match dur {
            rtttl_rs::Duration::Whole => Duration::Whole,
            rtttl_rs::Duration::Half => Duration::Half,
            rtttl_rs::Duration::Quarter => Duration::Quarter,
            rtttl_rs::Duration::Eighth
            | rtttl_rs::Duration::Sixteenth
            | rtttl_rs::Duration::ThirtySecond => Duration::Eighth,
        }
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Error parsing RTTTL format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtttlError {
    /// Missing colon separator in header.
    MissingColon,
    /// Invalid or empty input.
    InvalidInput,
    /// Melody exceeds maximum length.
    TooManyNotes,
}

impl From<rtttl_rs::ParseError> for RtttlError {
    fn from(err: rtttl_rs::ParseError) -> Self {
        match err {
            rtttl_rs::ParseError::MissingColon => RtttlError::MissingColon,
            rtttl_rs::ParseError::Empty
            | rtttl_rs::ParseError::InvalidSettings
            | rtttl_rs::ParseError::InvalidNote => RtttlError::InvalidInput,
        }
    }
}

/// A single note in a melody.
///
/// Byte encoding: `[NOTE:4][OCTAVE:2][DURATION:2]`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MelodyNote {
    pub note: Note,
    /// Relative octave (0-3). Add 4 to get real octave.
    pub octave: u8,
    pub duration: Duration,
}

impl MelodyNote {
    /// Decode a melody note from a byte.
    pub fn from_byte(b: u8) -> Self {
        Self {
            note: Note::from_nibble(b >> 4),
            octave: (b >> 2) & 0x03,
            duration: Duration::from_bits(b),
        }
    }

    /// Encode the note to a byte.
    pub fn to_byte(&self) -> u8 {
        (self.note.to_nibble() << 4) | ((self.octave & 0x03) << 2) | (self.duration as u8)
    }

    /// Get the real octave (4-7).
    pub fn real_octave(&self) -> u8 {
        self.octave + 4
    }

    /// Create a new melody note.
    pub const fn new(note: Note, octave: u8, duration: Duration) -> Self {
        Self {
            note,
            octave: if octave > 3 { 3 } else { octave },
            duration,
        }
    }

    /// Convert from rtttl-rs RtttlNote, adapting to AM32 constraints.
    fn from_rtttl(rtttl_note: rtttl_rs::RtttlNote) -> Self {
        let note = Note::from_rtttl(rtttl_note.note);
        let duration = Duration::from_rtttl(rtttl_note.duration);

        // AM32 supports octaves 4-7 (stored as 0-3)
        let am32_octave = if note == Note::Pause {
            0 // Octave doesn't matter for pause
        } else if rtttl_note.octave < 4 {
            0 // Transpose up to octave 4
        } else if rtttl_note.octave > 7 {
            3 // Transpose down to octave 7
        } else {
            rtttl_note.octave - 4
        };

        Self {
            note,
            octave: am32_octave,
            duration,
        }
    }
}

impl fmt::Display for MelodyNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.note == Note::Pause {
            write!(f, "P/{}", self.duration.symbol())
        } else {
            write!(
                f,
                "{}{}/{}",
                self.note,
                self.real_octave(),
                self.duration.symbol()
            )
        }
    }
}

/// A melody consisting of multiple notes (no_std compatible, fixed-size).
#[derive(Debug, Clone, Copy)]
pub struct Melody {
    notes: [MelodyNote; MELODY_MAX_NOTES],
    len: usize,
}

impl Melody {
    /// Create an empty melody.
    pub const fn new() -> Self {
        Self {
            notes: [MelodyNote::new(Note::Pause, 0, Duration::Quarter); MELODY_MAX_NOTES],
            len: 0,
        }
    }

    /// Parse a melody from raw EEPROM bytes.
    ///
    /// Returns `None` if the first byte is 0xFF (no melody).
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.is_empty() || data[0] == MELODY_EMPTY {
            return None;
        }

        let mut melody = Self::new();

        for &b in data.iter().take(MELODY_MAX_NOTES) {
            if b == 0x00 || b == 0xFF {
                break;
            }
            melody.notes[melody.len] = MelodyNote::from_byte(b);
            melody.len += 1;
        }

        Some(melody)
    }

    /// Encode the melody to raw EEPROM bytes.
    pub fn to_bytes(&self) -> [u8; MELODY_SIZE] {
        let mut buf = [0xFF; MELODY_SIZE];

        for i in 0..self.len {
            buf[i] = self.notes[i].to_byte();
        }

        // Add terminator if there are notes
        if self.len > 0 && self.len < MELODY_SIZE {
            buf[self.len] = 0x00;
        }

        buf
    }

    /// Check if the melody is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the number of notes.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Get the notes in the melody.
    pub fn notes(&self) -> &[MelodyNote] {
        &self.notes[..self.len]
    }

    /// Add a note to the melody. Returns false if melody is full.
    pub fn push(&mut self, note: MelodyNote) -> bool {
        if self.len < MELODY_MAX_NOTES {
            self.notes[self.len] = note;
            self.len += 1;
            true
        } else {
            false
        }
    }

    /// Get bytes to clear the melody (all 0xFF).
    pub const fn clear_bytes() -> [u8; MELODY_SIZE] {
        [0xFF; MELODY_SIZE]
    }

    /// Parse a melody from RTTTL (Ring Tone Text Transfer Language) format.
    ///
    /// RTTTL format: `name:d=duration,o=octave,b=bpm:notes`
    ///
    /// Example: `doom:b=420,o=4,d=32:8g4,8e4,8b3,4e,8p`
    ///
    /// Notes:
    /// - AM32 only supports octaves 4-7; lower octaves are transposed up
    /// - AM32 only supports whole/half/quarter/eighth; shorter notes become eighths
    /// - BPM is ignored (ESC has fixed tempo)
    /// - Dotted notes are treated as regular notes
    pub fn from_rtttl(rtttl: &str) -> Result<Self, RtttlError> {
        let parsed = rtttl_rs::Rtttl::parse(rtttl)?;
        let mut melody = Self::new();

        for note in parsed.notes() {
            let melody_note = MelodyNote::from_rtttl(note);
            if !melody.push(melody_note) {
                return Err(RtttlError::TooManyNotes);
            }
        }

        Ok(melody)
    }

    /// Get the melody name from RTTTL string (the part before the first colon).
    pub fn rtttl_name(rtttl: &str) -> Option<&str> {
        rtttl_rs::Rtttl::parse(rtttl).ok().map(|r| r.name())
    }
}

impl Default for Melody {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Melody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return write!(f, "(empty)");
        }

        for (i, note) in self.notes().iter().enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            write!(f, "{}", note)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_encoding_roundtrip() {
        for i in 0..=12 {
            let note = Note::from_nibble(i);
            assert_eq!(note.to_nibble(), i.min(12));
        }
    }

    #[test]
    fn duration_encoding_roundtrip() {
        for i in 0..=3 {
            let dur = Duration::from_bits(i);
            assert_eq!(dur as u8, i);
        }
    }

    #[test]
    fn melody_note_encoding() {
        // F4 quarter note: note=5, octave=0, duration=2
        // Byte: 0101 00 10 = 0x52
        let note = MelodyNote::from_byte(0x52);
        assert_eq!(note.note, Note::F);
        assert_eq!(note.octave, 0);
        assert_eq!(note.real_octave(), 4);
        assert_eq!(note.duration, Duration::Quarter);
        assert_eq!(note.to_byte(), 0x52);
    }

    #[test]
    fn melody_note_high_octave() {
        // A7 eighth note: note=9, octave=3, duration=3
        // Byte: 1001 11 11 = 0x9F
        let note = MelodyNote::from_byte(0x9F);
        assert_eq!(note.note, Note::A);
        assert_eq!(note.octave, 3);
        assert_eq!(note.real_octave(), 7);
        assert_eq!(note.duration, Duration::Eighth);
        assert_eq!(note.to_byte(), 0x9F);
    }

    #[test]
    fn melody_from_bytes_empty() {
        let data = [0xFF; 128];
        assert!(Melody::from_bytes(&data).is_none());
    }

    #[test]
    fn melody_from_bytes_simple() {
        let mut data = [0xFF; 128];
        data[0] = 0x52; // F4 quarter
        data[1] = 0x97; // A5 eighth
        data[2] = 0x00; // Terminator

        let melody = Melody::from_bytes(&data).unwrap();
        assert_eq!(melody.len(), 2);
        assert_eq!(melody.notes()[0].note, Note::F);
        assert_eq!(melody.notes()[1].note, Note::A);
    }

    #[test]
    fn melody_to_bytes_roundtrip() {
        let mut melody = Melody::new();
        melody.push(MelodyNote::new(Note::C, 0, Duration::Quarter));
        melody.push(MelodyNote::new(Note::E, 0, Duration::Quarter));
        melody.push(MelodyNote::new(Note::G, 0, Duration::Half));

        let bytes = melody.to_bytes();
        let parsed = Melody::from_bytes(&bytes).unwrap();

        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed.notes()[0].note, Note::C);
        assert_eq!(parsed.notes()[1].note, Note::E);
        assert_eq!(parsed.notes()[2].note, Note::G);
    }

    #[test]
    fn clear_bytes() {
        let bytes = Melody::clear_bytes();
        assert_eq!(bytes.len(), MELODY_SIZE);
        assert!(bytes.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn rtttl_parse_simple() {
        let rtttl = "test:d=4,o=5,b=120:c,d,e";
        let melody = Melody::from_rtttl(rtttl).unwrap();

        assert_eq!(melody.len(), 3);
        assert_eq!(melody.notes()[0].note, Note::C);
        assert_eq!(melody.notes()[0].real_octave(), 5);
        assert_eq!(melody.notes()[0].duration, Duration::Quarter);
        assert_eq!(melody.notes()[1].note, Note::D);
        assert_eq!(melody.notes()[2].note, Note::E);
    }

    #[test]
    fn rtttl_parse_with_durations() {
        let rtttl = "test:d=4,o=5,b=120:1c,2d,4e,8f";
        let melody = Melody::from_rtttl(rtttl).unwrap();

        assert_eq!(melody.len(), 4);
        assert_eq!(melody.notes()[0].duration, Duration::Whole);
        assert_eq!(melody.notes()[1].duration, Duration::Half);
        assert_eq!(melody.notes()[2].duration, Duration::Quarter);
        assert_eq!(melody.notes()[3].duration, Duration::Eighth);
    }

    #[test]
    fn rtttl_parse_with_octaves() {
        let rtttl = "test:d=4,o=5,b=120:c4,c5,c6,c7";
        let melody = Melody::from_rtttl(rtttl).unwrap();

        assert_eq!(melody.len(), 4);
        assert_eq!(melody.notes()[0].real_octave(), 4);
        assert_eq!(melody.notes()[1].real_octave(), 5);
        assert_eq!(melody.notes()[2].real_octave(), 6);
        assert_eq!(melody.notes()[3].real_octave(), 7);
    }

    #[test]
    fn rtttl_parse_sharps() {
        let rtttl = "test:d=4,o=5,b=120:c#,d#,f#,g#,a#";
        let melody = Melody::from_rtttl(rtttl).unwrap();

        assert_eq!(melody.len(), 5);
        assert_eq!(melody.notes()[0].note, Note::Cs);
        assert_eq!(melody.notes()[1].note, Note::Ds);
        assert_eq!(melody.notes()[2].note, Note::Fs);
        assert_eq!(melody.notes()[3].note, Note::Gs);
        assert_eq!(melody.notes()[4].note, Note::As);
    }

    #[test]
    fn rtttl_parse_pause() {
        let rtttl = "test:d=4,o=5,b=120:c,8p,e";
        let melody = Melody::from_rtttl(rtttl).unwrap();

        assert_eq!(melody.len(), 3);
        assert_eq!(melody.notes()[0].note, Note::C);
        assert_eq!(melody.notes()[1].note, Note::Pause);
        assert_eq!(melody.notes()[1].duration, Duration::Eighth);
        assert_eq!(melody.notes()[2].note, Note::E);
    }

    #[test]
    fn rtttl_parse_low_octave_transpose() {
        // Octave 3 should be transposed to octave 4
        let rtttl = "test:d=4,o=5,b=120:c3,e3";
        let melody = Melody::from_rtttl(rtttl).unwrap();

        assert_eq!(melody.len(), 2);
        assert_eq!(melody.notes()[0].real_octave(), 4); // Transposed from 3 to 4
        assert_eq!(melody.notes()[1].real_octave(), 4);
    }

    #[test]
    fn rtttl_parse_doom() {
        let rtttl = "doom:b=420,o=4,d=32:8g4,8e4,8b3,8e4,4e,8p,1a#3";
        let melody = Melody::from_rtttl(rtttl).unwrap();

        assert_eq!(Melody::rtttl_name(rtttl), Some("doom"));
        assert_eq!(melody.len(), 7);

        // 8g4 = G octave 4, eighth
        assert_eq!(melody.notes()[0].note, Note::G);
        assert_eq!(melody.notes()[0].real_octave(), 4);
        assert_eq!(melody.notes()[0].duration, Duration::Eighth);

        // 8b3 = Bb octave 3, transposed to 4
        assert_eq!(melody.notes()[2].note, Note::As); // 'b' = Bb in RTTTL
        assert_eq!(melody.notes()[2].real_octave(), 4);

        // 4e = E, quarter, default octave 4
        assert_eq!(melody.notes()[4].note, Note::E);
        assert_eq!(melody.notes()[4].duration, Duration::Quarter);

        // 8p = pause eighth
        assert_eq!(melody.notes()[5].note, Note::Pause);

        // 1a#3 = A# octave 3 transposed to 4, whole
        assert_eq!(melody.notes()[6].note, Note::As);
        assert_eq!(melody.notes()[6].real_octave(), 4);
        assert_eq!(melody.notes()[6].duration, Duration::Whole);
    }

    #[test]
    fn rtttl_parse_german_h_notation() {
        // H = B natural in German notation
        let rtttl = "test:d=4,o=5,b=120:h4,h5";
        let melody = Melody::from_rtttl(rtttl).unwrap();

        assert_eq!(melody.len(), 2);
        assert_eq!(melody.notes()[0].note, Note::B);
        assert_eq!(melody.notes()[0].real_octave(), 4);
        assert_eq!(melody.notes()[1].note, Note::B);
        assert_eq!(melody.notes()[1].real_octave(), 5);
    }

    #[test]
    fn rtttl_name_extraction() {
        assert_eq!(Melody::rtttl_name("doom:b=420,o=4:c"), Some("doom"));
        assert_eq!(Melody::rtttl_name("test:d=4:e"), Some("test"));
    }

    #[test]
    fn rtttl_missing_colon_error() {
        let result = Melody::from_rtttl("invalid");
        assert_eq!(result.err(), Some(RtttlError::MissingColon));
    }
}
