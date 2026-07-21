//! Turning raw PTY bytes into text something can reason about.
//!
//! Two callers want this and they want the same answer: the bus hands a peer's transcript to
//! another agent, and [`crate::terminal`] reads the tail of a node's own output to work out whether
//! the agent is waiting on its human. It lives here rather than in either of them because a second
//! copy of an escape-sequence parser is a second place for it to be subtly wrong.

/// Strip terminal escape noise, keep the readable text. Drops CSI (`ESC [ ... final`) and
/// OSC (`ESC ] ... BEL|ST`) sequences, other two-byte escapes, and bare control bytes except
/// `\n`/`\t`. Output is valid UTF-8 (lossy), so a later tail slice cannot split an escape.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            0x1b => {
                i += 1;
                match bytes.get(i) {
                    Some(b'[') => {
                        // CSI: params/intermediates until a final byte in 0x40..=0x7e.
                        i += 1;
                        while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                            i += 1;
                        }
                        i += 1;
                    }
                    Some(b']') => {
                        // OSC: runs until BEL or the ST terminator (ESC \).
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    Some(_) => i += 1, // other ESC x: drop the pair
                    None => {}
                }
            }
            b'\n' | b'\t' => {
                out.push(b);
                i += 1;
            }
            _ if b < 0x20 || b == 0x7f => i += 1, // CR, BEL, BS, DEL, ...
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Last `max` bytes of `s`, snapped forward to a char boundary so the slice stays valid UTF-8.
pub fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_go_and_the_words_stay() {
        // Colour codes and a CR vanish, the text does not.
        assert_eq!(strip_ansi(b"\x1b[31mred\x1b[0m\rline\n"), "redline\n");
        // An OSC title sequence ends at BEL and takes its payload with it.
        assert_eq!(strip_ansi(b"\x1b]0;my title\x07after"), "after");
        // A truncated escape at the very end cannot run off the end of the buffer.
        assert_eq!(strip_ansi(b"text\x1b"), "text");
    }

    #[test]
    fn tail_never_splits_a_character() {
        // Three bytes each. Cutting at 4 would land mid-character, so it snaps forward to 3
        // rather than producing a replacement char or panicking on a bad slice.
        let s = "日本語";
        assert_eq!(tail(s, 4), "語");
        assert_eq!(tail(s, 99), s);
    }
}
