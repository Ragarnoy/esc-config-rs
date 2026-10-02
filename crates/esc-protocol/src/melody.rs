//! Melody/music support for AM32 ESCs (Bluejay format).
//!
//! AM32 ESCs use the Bluejay startup melody format to play custom tunes.
//! The format stores frequency/duration pairs rather than musical note names.
//!
//! # Format
//!
//! ```text
//! [BPM_HI] [BPM_LO] [DEFAULT_OCTAVE] [DEFAULT_DURATION] [(count, freq_idx)...]
//! ```
//!
//! - Bytes 0-1: BPM as big-endian u16
//! - Byte 2: default octave
//! - Byte 3: default duration (RTTTL value: 1/2/4/8/16/32)
//! - Bytes 4+: pairs of (pulse_count, freq_index)
//!   - freq_index 0 = silence, pulse_count = duration in ms
//!   - freq_index != 0 = note, pulse_count = number of wave periods
//!   - A note can span multiple pairs if pulse_count > 255
//!
//! # RTTTL Support
//!
//! This module can parse RTTTL (Ring Tone Text Transfer Language) format
//! and convert to/from the Bluejay binary encoding.

use core::fmt;

/// Offset from config start to melody data.
pub const MELODY_OFFSET: usize = 48;

/// Size of melody data in bytes.
pub const MELODY_SIZE: usize = 128;

/// Header size (BPM:2 + octave:1 + duration:1).
const HEADER_SIZE: usize = 4;

/// Max data bytes after header.
const MAX_DATA: usize = MELODY_SIZE - HEADER_SIZE; // 124 bytes = 62 pairs

/// Maximum number of (count, freq_index) pairs.
pub const MELODY_MAX_NOTES: usize = MAX_DATA / 2; // 62

/// Value indicating no melody is configured (erased flash).
pub const MELODY_EMPTY: u8 = 0xFF;

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

/// A single entry in the Bluejay melody: (pulse_count, freq_index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MelodyEntry {
    /// Number of pulses (for notes) or milliseconds (for silence).
    pub count: u8,
    /// Frequency index. 0 = silence.
    /// Frequency = 1_000_000 / (24.72 * freq_index + 399.3) Hz
    pub freq_index: u8,
}

impl MelodyEntry {
    /// Get the frequency in Hz (0 for silence).
    pub fn frequency(&self) -> f32 {
        if self.freq_index == 0 {
            return 0.0;
        }
        1_000_000.0 / (24.72 * self.freq_index as f32 + 399.3)
    }

    /// Check if this is a silence/pause entry.
    pub const fn is_pause(&self) -> bool {
        self.freq_index == 0
    }
}

/// A melody in Bluejay format.
///
/// Contains a 4-byte header and up to 62 (count, freq_index) pairs.
#[derive(Debug, Clone, Copy)]
pub struct Melody {
    bpm: u16,
    default_octave: u8,
    default_duration: u8,
    entries: [MelodyEntry; MELODY_MAX_NOTES],
    len: usize,
}

impl Melody {
    /// Create an empty melody.
    pub const fn new() -> Self {
        Self {
            bpm: 120,
            default_octave: 5,
            default_duration: 4,
            entries: [MelodyEntry {
                count: 0,
                freq_index: 0,
            }; MELODY_MAX_NOTES],
            len: 0,
        }
    }

    /// Parse a melody from raw EEPROM bytes (Bluejay format).
    ///
    /// Returns `None` if the first byte is 0xFF (no melody / erased flash).
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < HEADER_SIZE || data[0] == MELODY_EMPTY {
            return None;
        }

        let bpm = ((data[0] as u16) << 8) | data[1] as u16;
        let default_octave = data[2];
        let default_duration = data[3];

        let mut entries = [MelodyEntry {
            count: 0,
            freq_index: 0,
        }; MELODY_MAX_NOTES];
        let mut len = 0;

        let mut i = HEADER_SIZE;
        while i + 1 < data.len() && len < MELODY_MAX_NOTES {
            let count = data[i];
            let freq = data[i + 1];
            // Terminate on zero count (no more entries)
            if count == 0 {
                break;
            }
            entries[len] = MelodyEntry {
                count,
                freq_index: freq,
            };
            len += 1;
            i += 2;
        }

        Some(Self {
            bpm,
            default_octave,
            default_duration,
            entries,
            len,
        })
    }

    /// Encode the melody to raw EEPROM bytes (Bluejay format).
    pub fn to_bytes(&self) -> [u8; MELODY_SIZE] {
        let mut buf = [0u8; MELODY_SIZE];

        // Header
        buf[0] = (self.bpm >> 8) as u8;
        buf[1] = self.bpm as u8;
        buf[2] = self.default_octave;
        buf[3] = self.default_duration;

        // Entries
        for i in 0..self.len {
            buf[HEADER_SIZE + i * 2] = self.entries[i].count;
            buf[HEADER_SIZE + i * 2 + 1] = self.entries[i].freq_index;
        }

        buf
    }

    /// Get bytes to clear the melody (all 0xFF).
    pub const fn clear_bytes() -> [u8; MELODY_SIZE] {
        [0xFF; MELODY_SIZE]
    }

    /// Check if the melody is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the number of (count, freq_index) pairs.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Get the BPM.
    pub const fn bpm(&self) -> u16 {
        self.bpm
    }

    /// Get the entries.
    pub fn entries(&self) -> &[MelodyEntry] {
        &self.entries[..self.len]
    }

    /// Count the number of distinct notes (merging split entries).
    pub fn note_count(&self) -> usize {
        let entries = self.entries();
        if entries.is_empty() {
            return 0;
        }

        let mut count = 1;
        for i in 1..entries.len() {
            // A continuation of the same note has the same freq_index
            // and the previous entry had count == 255
            if entries[i].freq_index != entries[i - 1].freq_index
                || entries[i - 1].count != 255
            {
                count += 1;
            }
        }
        count
    }

    /// Parse a melody from RTTTL (Ring Tone Text Transfer Language) format.
    ///
    /// Converts the RTTTL notes to Bluejay (count, freq_index) pairs.
    pub fn from_rtttl(rtttl: &str) -> Result<Self, RtttlError> {
        let parsed = rtttl_rs::Rtttl::parse(rtttl)?;
        let bpm = parsed.bpm();
        let default_octave = parsed.default_octave();
        let default_duration = parsed.default_duration().to_rtttl();

        let mut melody = Self {
            bpm,
            default_octave,
            default_duration,
            entries: [MelodyEntry {
                count: 0,
                freq_index: 0,
            }; MELODY_MAX_NOTES],
            len: 0,
        };

        for note in parsed.notes() {
            let duration_ms = note.duration_ms(bpm);

            if note.note == rtttl_rs::Note::Pause {
                // Silence: count = duration in ms, freq_index = 0
                // Split into multiple pairs if > 255ms
                let mut remaining = duration_ms;
                while remaining > 0 {
                    if melody.len >= MELODY_MAX_NOTES {
                        return Err(RtttlError::TooManyNotes);
                    }
                    let chunk = remaining.min(255);
                    melody.entries[melody.len] = MelodyEntry {
                        count: chunk as u8,
                        freq_index: 0,
                    };
                    melody.len += 1;
                    remaining -= chunk;
                }
            } else {
                // Calculate frequency from note + octave
                let freq = note_frequency(note.note, note.octave);
                let freq_index = freq_to_bluejay_index(freq);

                if freq_index == 0 || freq_index >= 255 {
                    // Frequency out of representable range, skip
                    continue;
                }

                // Calculate number of wave periods for the desired duration
                let period_ms = 1000.0 / freq;
                let pulses_needed = (duration_ms as f32 / period_ms + 0.5) as u32;

                // Split into multiple pairs if > 255 pulses
                let mut remaining = pulses_needed;
                while remaining > 0 {
                    if melody.len >= MELODY_MAX_NOTES {
                        return Err(RtttlError::TooManyNotes);
                    }
                    let chunk = remaining.min(255);
                    melody.entries[melody.len] = MelodyEntry {
                        count: chunk as u8,
                        freq_index: freq_index as u8,
                    };
                    melody.len += 1;
                    remaining -= chunk;
                }
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

        write!(f, "{}bpm, {} entries", self.bpm, self.len)?;

        // Show distinct notes with approximate pitch
        let entries = self.entries();
        let mut i = 0;
        let mut first = true;
        write!(f, " [")?;
        while i < entries.len() {
            if !first {
                write!(f, " ")?;
            }
            first = false;

            if entries[i].is_pause() {
                // Sum consecutive pause entries
                let mut total_ms: u32 = 0;
                while i < entries.len()
                    && entries[i].is_pause()
                {
                    total_ms += entries[i].count as u32;
                    i += 1;
                }
                write!(f, "P/{}ms", total_ms)?;
            } else {
                let freq = entries[i].frequency();
                // Sum pulses for same freq (split entries)
                let mut total_pulses: u32 = 0;
                let freq_idx = entries[i].freq_index;
                while i < entries.len() && entries[i].freq_index == freq_idx {
                    total_pulses += entries[i].count as u32;
                    // Only merge if previous was maxed out
                    if entries[i].count < 255 {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                let (name, octave) = freq_to_note_name(freq);
                write!(f, "{}{}", name, octave)?;
                let _ = total_pulses; // shown via entry count already
            }
        }
        write!(f, "]")?;

        Ok(())
    }
}

// =============================================================================
// Frequency helpers
// =============================================================================

/// Calculate frequency in Hz from a note and octave.
///
/// Uses equal temperament tuning with A4 = 440 Hz.
fn note_frequency(note: rtttl_rs::Note, octave: u8) -> f32 {
    if note == rtttl_rs::Note::Pause {
        return 0.0;
    }

    // Semitones from C4
    let semitone_index = match note {
        rtttl_rs::Note::C => 0,
        rtttl_rs::Note::Cs => 1,
        rtttl_rs::Note::D => 2,
        rtttl_rs::Note::Ds => 3,
        rtttl_rs::Note::E => 4,
        rtttl_rs::Note::F => 5,
        rtttl_rs::Note::Fs => 6,
        rtttl_rs::Note::G => 7,
        rtttl_rs::Note::Gs => 8,
        rtttl_rs::Note::A => 9,
        rtttl_rs::Note::As => 10,
        rtttl_rs::Note::B => 11,
        rtttl_rs::Note::Pause => return 0.0,
    };

    let semitones_from_c4 = semitone_index + (i32::from(octave) - 4) * 12;

    // C4 = 261.63 Hz, each semitone is 2^(1/12)
    261.63 * pow2_twelfths(semitones_from_c4)
}

/// Compute 2^(n/12) using a lookup table for common values.
fn pow2_twelfths(n: i32) -> f32 {
    // 2^(1/12) ≈ 1.05946309
    let base: f32 = 1.059_463_1;
    let mut result: f32 = 1.0;
    let abs_n = if n < 0 { -n } else { n } as u32;

    for _ in 0..abs_n {
        result *= base;
    }

    if n < 0 {
        1.0 / result
    } else {
        result
    }
}

/// Convert a frequency (Hz) to Bluejay freq_index.
///
/// freq = 1_000_000 / (24.72 * index + 399.3)
/// Solving for index: index = (1_000_000 / freq - 399.3) / 24.72
fn freq_to_bluejay_index(freq: f32) -> u32 {
    if freq <= 0.0 {
        return 0;
    }
    let index = (1_000_000.0 / (freq * 24.72) - 399.3 / 24.72 + 0.5) as i32;
    if index < 1 {
        1
    } else {
        index as u32
    }
}

/// Approximate a frequency to a note name and octave (for display only).
const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

fn freq_to_note_name(freq: f32) -> (&'static str, u8) {
    if freq <= 0.0 {
        return ("P", 0);
    }

    // Semitones from C4 (261.63 Hz)
    let semitones = 12.0 * log2(freq / 261.63);
    let semitones_rounded = (semitones + 0.5) as i32;

    let note_idx = ((semitones_rounded % 12) + 12) % 12;
    let octave = 4 + (semitones_rounded - note_idx) / 12;
    let octave = octave.clamp(0, 9) as u8;

    (NOTE_NAMES[note_idx as usize], octave)
}

/// Simple log2 approximation for no_std.
fn log2(x: f32) -> f32 {
    // Use the natural log identity: log2(x) = ln(x) / ln(2)
    // ln(x) ≈ using the bit representation trick
    #[allow(clippy::cast_sign_loss)]
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    let mantissa = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);

    // Polynomial approximation of log2(mantissa) for [1, 2)
    // log2(m) ≈ -1.6404 + m * (2.0466 + m * (-0.7382))
    let log2_mantissa = -1.640_4 + mantissa * (2.046_6 + mantissa * (-0.738_2));

    exponent as f32 + log2_mantissa
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn melody_from_bytes_empty() {
        let data = [0xFF; 128];
        assert!(Melody::from_bytes(&data).is_none());
    }

    #[test]
    fn melody_from_bytes_simple() {
        let mut data = [0u8; 128];
        // Header: 120 BPM, octave 5, quarter note default
        data[0] = 0; // BPM high
        data[1] = 120; // BPM low
        data[2] = 5; // octave
        data[3] = 4; // duration (quarter)
        // One entry: 100 pulses, freq_index 50
        data[4] = 100;
        data[5] = 50;

        let melody = Melody::from_bytes(&data).unwrap();
        assert_eq!(melody.bpm(), 120);
        assert_eq!(melody.len(), 1);
        assert_eq!(melody.entries()[0].count, 100);
        assert_eq!(melody.entries()[0].freq_index, 50);
    }

    #[test]
    fn melody_to_bytes_roundtrip() {
        let mut data = [0u8; 128];
        data[0] = 0;
        data[1] = 120;
        data[2] = 5;
        data[3] = 4;
        data[4] = 100;
        data[5] = 50;
        data[6] = 200;
        data[7] = 30;

        let melody = Melody::from_bytes(&data).unwrap();
        let output = melody.to_bytes();

        // Header and entries should match
        assert_eq!(&output[0..8], &data[0..8]);
        // Rest should be zeros
        assert!(output[8..].iter().all(|&b| b == 0));
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

        assert_eq!(melody.bpm(), 120);
        // Should have entries (one or more per note)
        assert!(melody.len() > 0);
        // All entries should have non-zero freq_index (no pauses)
        assert!(melody.entries().iter().all(|e| e.freq_index > 0));
    }

    #[test]
    fn rtttl_parse_with_pause() {
        let rtttl = "test:d=4,o=5,b=120:c,p,e";
        let melody = Melody::from_rtttl(rtttl).unwrap();

        // Should have pause entries (freq_index == 0)
        assert!(melody.entries().iter().any(|e| e.freq_index == 0));
    }

    #[test]
    fn freq_to_index_roundtrip() {
        // C5 = 523.25 Hz
        let freq = 523.25f32;
        let idx = freq_to_bluejay_index(freq);
        assert!(idx > 0);
        assert!(idx < 255);

        // Convert back and check it's close
        let entry = MelodyEntry {
            count: 1,
            freq_index: idx as u8,
        };
        let recovered = entry.frequency();
        let ratio = recovered / freq;
        assert!(
            (0.95..1.05).contains(&ratio),
            "frequency mismatch: {} vs {} (ratio {})",
            freq,
            recovered,
            ratio
        );
    }

    #[test]
    fn note_freq_a4() {
        let freq = note_frequency(rtttl_rs::Note::A, 4);
        // A4 should be close to 440 Hz
        assert!(
            (430.0..450.0).contains(&freq),
            "A4 frequency {} not close to 440",
            freq
        );
    }

    #[test]
    fn note_count_simple() {
        let rtttl = "test:d=4,o=5,b=120:c,d,e";
        let melody = Melody::from_rtttl(rtttl).unwrap();
        assert_eq!(melody.note_count(), 3);
    }

    #[test]
    fn display_format() {
        let rtttl = "test:d=4,o=5,b=120:c,p,e";
        let melody = Melody::from_rtttl(rtttl).unwrap();
        let s = std::format!("{}", melody);
        assert!(s.contains("120bpm"));
    }

    #[test]
    fn rtttl_doom() {
        let rtttl = "doom:b=420,o=4,d=32:8g4,8e4,8b3,4e,8p,1a#3";
        let melody = Melody::from_rtttl(rtttl).unwrap();
        // Should parse without error
        assert!(melody.len() > 0);
        assert_eq!(melody.bpm(), 420);
    }
}
